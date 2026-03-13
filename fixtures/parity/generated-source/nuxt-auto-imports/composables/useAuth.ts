export function useAuth() {
  const user = ref<string | null>(null);

  function login(name: string) {
    user.value = name;
  }

  function logout() {
    user.value = null;
  }

  return { user, login, logout };
}
