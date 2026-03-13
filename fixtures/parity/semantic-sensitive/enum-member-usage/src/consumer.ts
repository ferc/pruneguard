import { Status } from './status';

function isUserVisible(status: Status): boolean {
  return status === Status.Active || status === Status.Inactive;
}

export { isUserVisible };
