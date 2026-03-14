import { renderToString } from './render';

export async function render(url: string) {
  return renderToString(url);
}
