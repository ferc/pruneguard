import type { Foo, Bar } from './types';

function createFoo(name: string): Foo {
  return { id: crypto.randomUUID(), name };
}

function printBar(bar: Bar) {
  console.log(bar.label);
}

export { createFoo, printBar };
