//! Écriture d'archives ZIP (format des fichiers Office : .docx, .pptx, .xlsx).
//! Juste ce qu'il faut : fichiers compressés (deflate), sans chiffrement.

use std::io::Write;

use flate2::Compression;
use flate2::write::DeflateEncoder;

use crate::error::Result;

struct Entry {
    name: String,
    crc: u32,
    compressed: u32,
    size: u32,
    offset: u32,
    method: u16,
}

#[derive(Default)]
pub struct ZipWriter {
    out: Vec<u8>,
    entries: Vec<Entry>,
}

impl ZipWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute un fichier. Les images déjà compressées (JPEG, PNG) sont stockées telles quelles.
    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<()> {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let already_compressed = name.ends_with(".jpeg") || name.ends_with(".jpg") || name.ends_with(".png");
        let (method, body) = if already_compressed {
            (0u16, data.to_vec())
        } else {
            let mut enc = DeflateEncoder::new(Vec::with_capacity(data.len() / 3), Compression::new(6));
            enc.write_all(data)?;
            (8u16, enc.finish()?)
        };
        let too_big = || crate::error::HestiaError::Internal("fichier trop volumineux pour l'archive".into());
        let entry = Entry {
            name: name.to_string(),
            crc: crc.sum(),
            compressed: u32::try_from(body.len()).map_err(|_| too_big())?,
            size: u32::try_from(data.len()).map_err(|_| too_big())?,
            offset: u32::try_from(self.out.len()).map_err(|_| too_big())?,
            method,
        };
        // En-tête local.
        let o = &mut self.out;
        o.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        o.extend_from_slice(&20u16.to_le_bytes()); // version
        o.extend_from_slice(&0x0800u16.to_le_bytes()); // noms en UTF-8
        o.extend_from_slice(&entry.method.to_le_bytes());
        o.extend_from_slice(&0u16.to_le_bytes()); // heure
        o.extend_from_slice(&0x0021u16.to_le_bytes()); // date (1980-01-01)
        o.extend_from_slice(&entry.crc.to_le_bytes());
        o.extend_from_slice(&entry.compressed.to_le_bytes());
        o.extend_from_slice(&entry.size.to_le_bytes());
        o.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        o.extend_from_slice(&0u16.to_le_bytes());
        o.extend_from_slice(entry.name.as_bytes());
        o.extend_from_slice(&body);
        self.entries.push(entry);
        Ok(())
    }

    pub fn finish(mut self) -> Vec<u8> {
        let start = self.out.len() as u32;
        for e in &self.entries {
            let o = &mut self.out;
            o.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            o.extend_from_slice(&20u16.to_le_bytes()); // créé par
            o.extend_from_slice(&20u16.to_le_bytes()); // version nécessaire
            o.extend_from_slice(&0x0800u16.to_le_bytes());
            o.extend_from_slice(&e.method.to_le_bytes());
            o.extend_from_slice(&0u16.to_le_bytes());
            o.extend_from_slice(&0x0021u16.to_le_bytes());
            o.extend_from_slice(&e.crc.to_le_bytes());
            o.extend_from_slice(&e.compressed.to_le_bytes());
            o.extend_from_slice(&e.size.to_le_bytes());
            o.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            o.extend_from_slice(&[0; 8]); // extra, commentaire, disque, attributs internes (2 octets chacun)
            o.extend_from_slice(&0u32.to_le_bytes()); // attributs externes
            o.extend_from_slice(&e.offset.to_le_bytes());
            o.extend_from_slice(e.name.as_bytes());
        }
        let size = self.out.len() as u32 - start;
        let count = self.entries.len() as u16;
        let o = &mut self.out;
        o.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        o.extend_from_slice(&[0; 4]);
        o.extend_from_slice(&count.to_le_bytes());
        o.extend_from_slice(&count.to_le_bytes());
        o.extend_from_slice(&size.to_le_bytes());
        o.extend_from_slice(&start.to_le_bytes());
        o.extend_from_slice(&0u16.to_le_bytes());
        self.out
    }
}

/// Texte sûr pour du XML (échappé, sans caractères interdits).
pub fn xml(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\t' | '\n' => s.push(ch),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => s.push(c),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_is_readable_by_the_image_crate_zip_layout() {
        let mut z = ZipWriter::new();
        z.add("a.txt", b"bonjour bonjour bonjour").unwrap_or_default();
        z.add("b/c.png", &[1, 2, 3]).unwrap_or_default();
        let bytes = z.finish();
        assert!(bytes.starts_with(b"PK\x03\x04"));
        // Fin de répertoire central : 2 entrées.
        let end = &bytes[bytes.len() - 22..];
        assert_eq!(&end[..4], b"PK\x05\x06");
        assert_eq!(u16::from_le_bytes([end[10], end[11]]), 2);
        // Le répertoire central commence à l'endroit annoncé et finit juste avant la fin.
        let size = u32::from_le_bytes([end[12], end[13], end[14], end[15]]) as usize;
        let start = u32::from_le_bytes([end[16], end[17], end[18], end[19]]) as usize;
        assert_eq!(start + size, bytes.len() - 22);
        assert_eq!(&bytes[start..start + 4], b"PK");
        // Taille d'une entrée : 46 octets + nom.
        let second = start + 46 + "a.txt".len();
        assert_eq!(&bytes[second..second + 4], b"PK");
    }

    #[test]
    fn xml_is_escaped() {
        assert_eq!(xml("a<b & \"c\"\u{1}"), "a&lt;b &amp; &quot;c&quot;");
    }
}
