<template>
  <div id="app">
    <h1>{{ title }}</h1>
    <MyVue />
    <my-counter v-if="showCounter" />
    <UnimportedWidget v-for="item in items" :key="item.id" />
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue';
import MyVue from './MyVue.vue';
import { greet } from '../utils';

const title = greet('App');
const showCounter = ref(true);
const items = ref([{ id: 1 }, { id: 2 }]);
</script>

<style scoped>
.app {
  font-family: sans-serif;
}
</style>
