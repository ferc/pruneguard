// This file uses the ambient AppConfig type from types.d.ts.
// types.d.ts should NOT be flagged as unused (ambient exclusion).
// unused.ts has no importers and SHOULD be flagged.
const value: AppConfig = { debug: false };

console.log(value);
