// Préférences de l'utilisateur (thème, langue), stockées localement.
// Le stockage peut être indisponible : l'appli fonctionne alors sans.

const PREFIX = "hestia.";

export function readSetting(key: string): string | null {
  try {
    return window.localStorage.getItem(PREFIX + key);
  } catch {
    return null;
  }
}

export function writeSetting(key: string, value: string): void {
  try {
    window.localStorage.setItem(PREFIX + key, value);
  } catch {
    /* préférence non mémorisée : sans conséquence */
  }
}
