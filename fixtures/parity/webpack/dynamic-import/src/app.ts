export async function loadPage(name: string) {
  if (name === 'dashboard') {
    return import(/* webpackChunkName: "dashboard" */ './lazy-dashboard');
  }
  return import(/* webpackChunkName: "settings", webpackMode: "lazy" */ './lazy-settings');
}
