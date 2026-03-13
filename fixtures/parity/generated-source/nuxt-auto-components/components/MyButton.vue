<template>
  <button class="my-button">
    <slot />
  </button>
</template>

<script setup lang="ts">
defineProps<{
  variant?: 'primary' | 'secondary';
}>();
</script>
