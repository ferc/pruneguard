// Imports referencing files that don't exist, creating truly unresolved specifiers.
import { a } from "./missing-a";
import { b } from "./missing-b";
import { c } from "./missing-c";
import { d } from "./missing-d";
import { e } from "./missing-e";
import { f } from "./missing-f";
import { g } from "./missing-g";
import { h } from "./missing-h";

console.log(a, b, c, d, e, f, g, h);
