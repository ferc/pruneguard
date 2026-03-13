<template>
  <div>{{ message }}</div>
</template>

<script setup lang="ts">
import { formatMessage } from './utils';

const message = formatMessage('hello');
</script>
