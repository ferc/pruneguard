import { Status } from './status';

function checkStatus(s: Status) {
  if (s === Status.Active) return true;
  if (s === Status.Inactive) return false;
  return false;
}

export { checkStatus };
