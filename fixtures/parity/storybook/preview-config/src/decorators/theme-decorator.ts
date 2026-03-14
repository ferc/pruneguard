export function withTheme(storyFn: () => string) {
  return `<div class="theme-wrapper">${storyFn()}</div>`;
}
