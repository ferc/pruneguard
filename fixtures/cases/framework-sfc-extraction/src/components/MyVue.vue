<template>
  <div>{{ message }}</div>
</template>

<script setup lang="ts">
import { greet } from '../utils';

const message = greet('Vue');
</script>
