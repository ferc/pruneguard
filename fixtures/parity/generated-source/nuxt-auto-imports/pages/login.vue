<template>
  <div>
    <button @click="login('admin')">Login</button>
    <span>{{ user }}</span>
  </div>
</template>

<script setup lang="ts">
// useAuth is auto-imported by Nuxt — no explicit import statement
const { user, login } = useAuth();
</script>
