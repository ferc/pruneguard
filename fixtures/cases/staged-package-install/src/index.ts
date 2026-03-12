import { merge } from "lodash";

export function combine(a: object, b: object): object {
  return merge({}, a, b);
}
