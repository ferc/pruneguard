import { groupBy } from "lodash";

const data = [
  { type: "a", value: 1 },
  { type: "b", value: 2 },
];

console.log(groupBy(data, "type"));
