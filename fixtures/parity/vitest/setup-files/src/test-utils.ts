export function cleanup() {
  document.body.innerHTML = '';
}

export function render(html: string) {
  document.body.innerHTML = html;
}
