export async function getSession(request: Request) {
  return { user: { name: 'Alice' } };
}
