import { Color } from './color';

function paint(c: Color) {
  if (c === Color.Red) return '#ff0000';
  if (c === Color.Blue) return '#0000ff';
  return '#000000';
}

export { paint };
