//! Conversions avec la suite bureautique installée sur le PC.
//!
//! - **Microsoft Office** (Word, PowerPoint, Excel) est piloté en
//!   arrière-plan par automatisation COM, via un petit script PowerShell
//!   intégré à l'exécutable : aucune fenêtre, macros désactivées, fichier
//!   ouvert en lecture seule. Le résultat est celui de « Enregistrer en PDF »
//!   dans Office.
//! - **LibreOffice**, s'il est installé, prend le relais quand Office manque.
//!
//! Rien ne sort du PC. Les chemins sont transmis par variables
//! d'environnement (jamais insérés dans le script), et l'instance d'Office
//! démarrée pour la conversion est arrêtée en cas d'annulation ou de blocage.

use std::io::{BufRead, BufReader, Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::engine::PdfEngine;
use crate::error::{HestiaError, Result};
use crate::job::Job;
use crate::output::{OutputFile, ensure_not_input, write_atomic};

/// Famille de document bureautique.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DocKind {
    Word,
    Powerpoint,
    Excel,
}

pub const WORD_EXTENSIONS: &[&str] = &["doc", "docx", "docm", "dot", "dotx", "dotm", "odt", "rtf"];
pub const POWERPOINT_EXTENSIONS: &[&str] = &["ppt", "pptx", "pptm", "pps", "ppsx", "pot", "potx", "odp"];
pub const EXCEL_EXTENSIONS: &[&str] = &["xls", "xlsx", "xlsm", "xlsb", "ods", "csv"];

impl DocKind {
    pub fn of(path: &Path) -> Option<DocKind> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        let ext = ext.as_str();
        if WORD_EXTENSIONS.contains(&ext) {
            Some(DocKind::Word)
        } else if POWERPOINT_EXTENSIONS.contains(&ext) {
            Some(DocKind::Powerpoint)
        } else if EXCEL_EXTENSIONS.contains(&ext) {
            Some(DocKind::Excel)
        } else {
            None
        }
    }

    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            DocKind::Word => WORD_EXTENSIONS,
            DocKind::Powerpoint => POWERPOINT_EXTENSIONS,
            DocKind::Excel => EXCEL_EXTENSIONS,
        }
    }

    fn script_mode(self) -> &'static str {
        match self {
            DocKind::Word => "word",
            DocKind::Powerpoint => "powerpoint",
            DocKind::Excel => "excel",
        }
    }
}

/// Logiciels disponibles pour les conversions.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficeStatus {
    pub word: bool,
    pub powerpoint: bool,
    pub excel: bool,
    pub libre_office: bool,
    #[serde(skip)]
    soffice: Option<PathBuf>,
}

/// Logiciel retenu pour une conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Converter {
    Microsoft,
    LibreOffice(PathBuf),
}

impl OfficeStatus {
    /// Recherche Office (inscription COM) et LibreOffice. Rapide (< 0,2 s).
    pub fn detect() -> Self {
        let soffice = find_soffice();
        Self {
            word: com_registered("Word.Application"),
            powerpoint: com_registered("PowerPoint.Application"),
            excel: com_registered("Excel.Application"),
            libre_office: soffice.is_some(),
            soffice,
        }
    }

    pub fn microsoft(&self, kind: DocKind) -> bool {
        match kind {
            DocKind::Word => self.word,
            DocKind::Powerpoint => self.powerpoint,
            DocKind::Excel => self.excel,
        }
    }

    /// Office d'abord (rendu identique à l'original), sinon LibreOffice.
    pub fn converter(&self, kind: DocKind) -> Option<Converter> {
        if self.microsoft(kind) { Some(Converter::Microsoft) } else { self.soffice.clone().map(Converter::LibreOffice) }
    }
}

/// Imprimante par défaut hors ligne, s'il y en a une. Word et Excel s'en
/// servent pour mettre en page : hors ligne, ils peuvent ne plus répondre.
pub fn offline_default_printer() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let script = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; $p = Get-CimInstance Win32_Printer -Filter 'Default = TRUE'; if ($p -and ($p.WorkOffline -or $p.PrinterStatus -eq 7)) { $p.Name }";
    let mut cmd = Command::new(powershell());
    cmd.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    hide_window(&mut cmd);
    let out = cmd.output().ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// Délai maximal d'une conversion (au-delà, Office est considéré bloqué).
const TIMEOUT: Duration = Duration::from_secs(180);

// ---------------------------------------------------------------------------
// Word / PowerPoint / Excel → PDF
// ---------------------------------------------------------------------------

/// Convertit un document Word, PowerPoint ou Excel en PDF.
pub fn office_to_pdf(
    engine: &PdfEngine,
    status: &OfficeStatus,
    input: &Path,
    output: &Path,
    job: &Job,
) -> Result<OutputFile> {
    ensure_not_input(output, [input])?;
    let kind =
        DocKind::of(input).ok_or_else(|| HestiaError::OfficeFailed("type de document non pris en charge".into()))?;
    let converter = status.converter(kind).ok_or(HestiaError::OfficeMissing)?;
    job.check()?;
    let tmp = tempfile::Builder::new().prefix("hestia-office-").tempdir()?;
    let produced = match &converter {
        Converter::Microsoft => {
            let out = tmp.path().join("resultat.pdf");
            run_office_script(kind.script_mode(), input, &out, job)?;
            out
        }
        Converter::LibreOffice(soffice) => run_libreoffice(soffice, input, tmp.path(), job)?,
    };
    let bytes =
        std::fs::read(&produced).map_err(|_| HestiaError::OfficeFailed("aucun fichier n'a été produit".into()))?;
    job.check()?;
    // Le PDF produit doit s'ouvrir et avoir au moins une page.
    let pages = page_count(engine, bytes.clone())?;
    if pages == 0 {
        return Err(HestiaError::OfficeFailed("le document est vide".into()));
    }
    write_atomic(output, |f| f.write_all(&bytes).map_err(Into::into))
}

/// Nombre de pages d'un PDF en mémoire (vérifie qu'il est lisible).
pub(crate) fn page_count(engine: &PdfEngine, bytes: Vec<u8>) -> Result<u32> {
    engine.run(move |ctx| {
        let doc = ctx
            .pdfium()
            .load_pdf_from_byte_vec(bytes, None)
            .map_err(|e| HestiaError::OfficeFailed(format!("le PDF produit est illisible ({e:?})")))?;
        Ok(doc.pages().len() as u32)
    })
}

// ---------------------------------------------------------------------------
// Microsoft Office (COM)
// ---------------------------------------------------------------------------

/// Script exécuté par Windows PowerShell. Variables d'entrée : HESTIA_MODE,
/// HESTIA_IN, HESTIA_OUT. Sortie : « PID:n » pour chaque instance d'Office
/// démarrée, puis « OK » ou « ERR:message ».
const OFFICE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$src = $env:HESTIA_IN
$dst = $env:HESTIA_OUT
$mode = $env:HESTIA_MODE
$noPassword = 'hestia-aucun-mot-de-passe'
$missing = [Type]::Missing
function Say([string]$line) { [Console]::Out.WriteLine($line); [Console]::Out.Flush() }
$script:officePids = @()
function Start-Office([string]$progId, [string]$process) {
  $before = @(Get-Process -Name $process -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  $instance = New-Object -ComObject $progId
  Get-Process -Name $process -ErrorAction SilentlyContinue |
    Where-Object { $before -notcontains $_.Id } |
    ForEach-Object { $script:officePids += $_.Id; Say "PID:$($_.Id)" }
  return @($instance, ($before.Count -gt 0))
}
$app = $null; $file = $null; $quit = $true; $code = 0
try {
  switch ($mode) {
    'word' {
      $app = (Start-Office 'Word.Application' 'WINWORD')[0]
      $app.Visible = $false
      $app.DisplayAlerts = 0
      $app.AutomationSecurity = 3
      $app.Options.ConfirmConversions = $false
      # Open(FileName, ConfirmConversions, ReadOnly, AddToRecentFiles, PasswordDocument)
      $file = $app.Documents.Open($src, $false, $true, $false, $noPassword)
      # PDF avec signets pour les titres, balisage d'accessibilité.
      $export = { $file.ExportAsFixedFormat($dst, 17, $false, 0, 0, 1, 1, 0, $true, $true, 1, $true, $true, $false) }
      try { & $export } catch {
        # Word exige une imprimante en état de marche pour mettre en page.
        # Si celle par défaut est hors ligne, on prend une imprimante
        # virtuelle de Windows, pour cette instance de Word seulement
        # (l'imprimante par défaut de Windows n'est pas changée).
        $first = $_
        $done = $false
        foreach ($printer in 'Microsoft Print to PDF', 'Microsoft XPS Document Writer') {
          try {
            [void][System.__ComObject].InvokeMember('FilePrintSetup', [Reflection.BindingFlags]::InvokeMethod, $null,
              $app.WordBasic, @($printer, 1), $null, $null, [string[]]@('Printer', 'DoNotSetAsSysDefault'))
            & $export
            $done = $true
            break
          } catch {}
        }
        if (-not $done) { throw $first }
      }
    }
    'excel' {
      $app = (Start-Office 'Excel.Application' 'EXCEL')[0]
      $app.Visible = $false
      $app.DisplayAlerts = $false
      $app.AskToUpdateLinks = $false
      $app.EnableEvents = $false
      $app.AutomationSecurity = 3
      # Open(FileName, UpdateLinks, ReadOnly, Format, Password)
      $file = $app.Workbooks.Open($src, 0, $true, $missing, $noPassword)
      $file.ExportAsFixedFormat(0, $dst)
    }
    'powerpoint' {
      $started = Start-Office 'PowerPoint.Application' 'POWERPNT'
      $app = $started[0]
      # PowerPoint n'a qu'une instance : si l'utilisateur l'avait ouvert, on ne le ferme pas.
      $quit = -not $started[1]
      $app.DisplayAlerts = 1
      $app.AutomationSecurity = 3
      # Open(FileName, ReadOnly, Untitled, WithWindow)
      $file = $app.Presentations.Open($src, -1, 0, 0)
      $file.SaveAs($dst, 32)
    }
    default { throw "mode inconnu : $mode" }
  }
  Say 'OK'
} catch {
  $message = $_.Exception.Message -replace '[\r\n]+', ' '
  Say "ERR:$message"
  $code = 1
} finally {
  if ($null -ne $file) {
    try {
      if ($mode -eq 'excel') { $file.Close($false) } elseif ($mode -eq 'powerpoint') { $file.Close() } else { $file.Close(0) }
    } catch {}
    try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($file) } catch {}
  }
  if ($null -ne $app) {
    if ($quit) { try { $app.Quit() } catch {} }
    try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($app) } catch {}
  }
  $app = $null; $file = $null; $started = $null
  [GC]::Collect()
  [GC]::WaitForPendingFinalizers()
  # On attend la fin de l'instance démarrée : la conversion suivante ne
  # doit pas s'accrocher à un Office en train de se fermer.
  if ($quit) { foreach ($id in $script:officePids) { try { Wait-Process -Id $id -Timeout 20 -ErrorAction SilentlyContinue } catch {} } }
}
exit $code
"#;

fn run_office_script(mode: &str, input: &Path, output: &Path, job: &Job) -> Result<()> {
    let mut cmd = Command::new(powershell());
    cmd.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand"])
        .arg(encode_command(OFFICE_SCRIPT))
        .env("HESTIA_MODE", mode)
        .env("HESTIA_IN", input)
        .env("HESTIA_OUT", output);
    let run = supervise(cmd, job)?;
    if run.success && run.lines.iter().any(|l| l == "OK") && output.is_file() {
        return Ok(());
    }
    let message = run
        .lines
        .iter()
        .find_map(|l| l.strip_prefix("ERR:"))
        .map(str::to_string)
        .unwrap_or_else(|| powershell_errors(&run.stderr));
    Err(office_error(&message))
}

/// Erreurs de PowerShell, qui arrivent parfois sérialisées (« #< CLIXML »).
fn powershell_errors(stderr: &str) -> String {
    if !stderr.trim_start().starts_with("#< CLIXML") {
        return stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
    }
    let mut out = String::new();
    for part in stderr.split("<S S=\"Error\">").skip(1) {
        if let Some(text) = part.split("</S>").next() {
            out.push_str(text);
        }
    }
    out.replace("_x000D_", "")
        .replace("_x000A_", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Traduit le message d'Office en erreur compréhensible.
fn office_error(message: &str) -> HestiaError {
    let lower = message.to_lowercase();
    if lower.contains("mot de passe") || lower.contains("password") || lower.contains("kennwort") {
        HestiaError::OfficeProtected
    } else if message.is_empty() {
        HestiaError::OfficeFailed("la conversion n'a rien produit".into())
    } else {
        HestiaError::OfficeFailed(message.chars().take(300).collect())
    }
}

// ---------------------------------------------------------------------------
// LibreOffice
// ---------------------------------------------------------------------------

/// Convertit avec LibreOffice ; renvoie le fichier produit dans `dir`.
fn run_libreoffice(soffice: &Path, input: &Path, dir: &Path, job: &Job) -> Result<PathBuf> {
    // Profil temporaire : n'interfère pas avec un LibreOffice déjà ouvert.
    let profile = dir.join("profil");
    let out_dir = dir.join("sortie");
    std::fs::create_dir_all(&out_dir)?;
    let mut cmd = Command::new(soffice);
    cmd.arg(format!("-env:UserInstallation={}", file_url(&profile))).args([
        "--headless",
        "--invisible",
        "--norestore",
        "--nolockcheck",
        "--nodefault",
        "--nologo",
    ]);
    cmd.args(["--convert-to", "pdf", "--outdir"]).arg(&out_dir).arg(input);
    let run = supervise(cmd, job)?;
    let ext = "pdf";
    let produced = std::fs::read_dir(&out_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)));
    match produced {
        Some(p) if run.success || p.is_file() => Ok(p),
        _ => {
            let detail = run.stderr.lines().chain(run.lines.iter().map(String::as_str)).find(|l| !l.trim().is_empty());
            Err(office_error(detail.unwrap_or("").trim()))
        }
    }
}

/// URL `file:///` d'un chemin (pour l'option de profil de LibreOffice).
fn file_url(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let mut url = String::from(if raw.starts_with('/') { "file://" } else { "file:///" });
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

fn find_soffice() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(dir) = std::env::var_os(var) {
                candidates.push(PathBuf::from(dir).join("LibreOffice").join("program").join("soffice.exe"));
            }
        }
    } else {
        candidates.extend(
            ["/usr/bin/soffice", "/usr/local/bin/soffice", "/Applications/LibreOffice.app/Contents/MacOS/soffice"]
                .map(PathBuf::from),
        );
    }
    candidates.into_iter().find(|p| p.is_file())
}

// ---------------------------------------------------------------------------
// Processus
// ---------------------------------------------------------------------------

struct Run {
    success: bool,
    lines: Vec<String>,
    stderr: String,
}

/// Lance la commande sans fenêtre, en surveillant l'annulation et le délai.
/// En cas d'arrêt, le processus et les instances d'Office signalées
/// (« PID:n ») sont terminés.
fn supervise(mut cmd: Command, job: &Job) -> Result<Run> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| HestiaError::OfficeFailed(format!("lancement impossible : {e}")))?;

    let (tx, rx) = mpsc::channel::<String>();
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(std::result::Result::ok) {
                let _ = tx.send(line.trim().to_string());
            }
        });
    }
    let stderr_reader = child.stderr.take().map(|mut err| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            s
        })
    });

    let start = Instant::now();
    let mut lines = Vec::new();
    let mut pids = Vec::new();
    let collect = |line: String, lines: &mut Vec<String>, pids: &mut Vec<u32>| match line.strip_prefix("PID:") {
        Some(pid) => pids.extend(pid.trim().parse::<u32>().ok()),
        None => lines.push(line),
    };
    let status = loop {
        while let Ok(line) = rx.try_recv() {
            collect(line, &mut lines, &mut pids);
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if job.check().is_err() {
            stop(&mut child, &pids);
            return Err(HestiaError::Cancelled);
        }
        if start.elapsed() > TIMEOUT {
            stop(&mut child, &pids);
            return Err(HestiaError::OfficeTimeout);
        }
        std::thread::sleep(Duration::from_millis(120));
    };
    // Lignes restantes, une fois la sortie fermée.
    while let Ok(line) = rx.recv_timeout(Duration::from_secs(2)) {
        collect(line, &mut lines, &mut pids);
    }
    let stderr = stderr_reader.and_then(|h| h.join().ok()).unwrap_or_default();
    Ok(Run { success: status.success(), lines, stderr })
}

fn stop(child: &mut Child, pids: &[u32]) {
    let _ = child.kill();
    let _ = child.wait();
    for pid in pids {
        let mut kill = if cfg!(windows) {
            let mut c = Command::new(system32("taskkill.exe"));
            c.args(["/PID", &pid.to_string(), "/T", "/F"]);
            c
        } else {
            let mut c = Command::new("kill");
            c.args(["-9", &pid.to_string()]);
            c
        };
        kill.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        hide_window(&mut kill);
        let _ = kill.status();
    }
}

#[cfg(windows)]
fn hide_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_cmd: &mut Command) {}

/// Outil système de Windows, par chemin absolu (jamais via le PATH).
fn system32(exe: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32").join(exe)
}

fn powershell() -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe")
}

/// L'application COM est-elle inscrite (Office installé) ?
fn com_registered(prog_id: &str) -> bool {
    if !cfg!(windows) {
        return false;
    }
    let mut cmd = Command::new(system32("reg.exe"));
    cmd.args(["query", &format!(r"HKCR\{prog_id}\CLSID")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_window(&mut cmd);
    cmd.status().is_ok_and(|s| s.success())
}

/// Script encodé pour `-EncodedCommand` (UTF-16LE puis base64).
fn encode_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&bytes)
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn kinds_follow_extensions() {
        assert_eq!(DocKind::of(Path::new("a.DOCX")), Some(DocKind::Word));
        assert_eq!(DocKind::of(Path::new("b.pptx")), Some(DocKind::Powerpoint));
        assert_eq!(DocKind::of(Path::new("c.csv")), Some(DocKind::Excel));
        assert_eq!(DocKind::of(Path::new("d.pdf")), None);
    }

    #[test]
    fn file_urls_are_escaped() {
        assert_eq!(file_url(Path::new(r"C:\Temp\mon profil")), "file:///C:/Temp/mon%20profil");
    }

    #[test]
    fn office_messages_are_classified() {
        assert!(matches!(office_error("Le mot de passe est incorrect."), HestiaError::OfficeProtected));
        assert!(matches!(office_error(""), HestiaError::OfficeFailed(_)));
    }
}
