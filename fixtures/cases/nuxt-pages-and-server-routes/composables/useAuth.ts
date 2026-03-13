export function useAuth() {
  const user = ref(null);
  return { user };
}
