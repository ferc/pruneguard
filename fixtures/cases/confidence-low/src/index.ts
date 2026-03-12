// Multiple unresolvable imports create high unresolved pressure
import { a } from "unknown-pkg-a";
import { b } from "unknown-pkg-b";
import { c } from "unknown-pkg-c";
import { helper } from "./lib";

console.log(a, b, c, helper());
