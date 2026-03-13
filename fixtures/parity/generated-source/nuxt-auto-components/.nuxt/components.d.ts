export { default as MyButton } from '../components/MyButton.vue';

declare module 'vue' {
  export interface GlobalComponents {
    MyButton: typeof import('../components/MyButton.vue')['default'];
  }
}
