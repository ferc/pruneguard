import { UserService } from './service';

const svc = new UserService();
const user = svc.getById('1');
const users = svc.list();

console.log(user, users);
