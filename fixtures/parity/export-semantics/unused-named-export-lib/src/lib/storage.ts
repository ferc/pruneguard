// Library with multiple exports — only saveProgress and loadProgress are consumed.
// getHighestLevel and clearAllData are never imported by anyone.

export function saveProgress(data: { level: number }): void {
  localStorage.setItem('progress', JSON.stringify(data));
}

export function loadProgress(): { level: number } {
  const raw = localStorage.getItem('progress');
  return raw ? JSON.parse(raw) : { level: 0 };
}

export function getHighestLevel(): number {
  const data = loadProgress();
  return data.level;
}

export function clearAllData(): void {
  localStorage.removeItem('progress');
}
