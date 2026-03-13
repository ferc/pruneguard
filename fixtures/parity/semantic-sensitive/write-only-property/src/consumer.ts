import { AppState } from './state';

const state = new AppState();

// count is both written and read
state.count = 42;
console.log(state.count);

// lastError is only written, never read
state.lastError = 'something went wrong';
state.lastError = null;
