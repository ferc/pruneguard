export class UserService {
  getById(id: string) {
    return { id, name: 'User' };
  }

  list() {
    return [{ id: '1', name: 'Alice' }, { id: '2', name: 'Bob' }];
  }

  archive(id: string) {
    console.log(`Archiving user ${id}`);
  }

  export(format: 'csv' | 'json') {
    console.log(`Exporting in ${format}`);
  }
}
