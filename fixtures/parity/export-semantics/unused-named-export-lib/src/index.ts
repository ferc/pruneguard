import { saveProgress, loadProgress } from './lib/storage';

export function app() {
  const data = loadProgress();
  saveProgress({ level: data.level + 1 });
}
