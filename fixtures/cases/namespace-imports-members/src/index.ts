import * as utils from "./utils";

// Only utils.foo is accessed. utils.bar should be flagged as an unused export.
console.log(utils.foo());
