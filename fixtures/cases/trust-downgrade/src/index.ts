// Many unresolvable imports create high unresolved pressure,
// which should cause trust scoring to downgrade confidence.
import { a } from "nonexistent-pkg-alpha";
import { b } from "nonexistent-pkg-beta";
import { c } from "nonexistent-pkg-gamma";
import { d } from "nonexistent-pkg-delta";
import { e } from "nonexistent-pkg-epsilon";
import { helper } from "./used";

console.log(a, b, c, d, e, helper());
