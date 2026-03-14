export function isAuthenticated(request: any): boolean {
  return request.cookies.has('session');
}
