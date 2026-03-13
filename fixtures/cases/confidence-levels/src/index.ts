import { helper } from "./used";
import { load } from "unknown-dynamic-loader";

// One unresolvable import creates moderate unresolved pressure.
const plugin = load("./plugins");

console.log(helper(), plugin);
