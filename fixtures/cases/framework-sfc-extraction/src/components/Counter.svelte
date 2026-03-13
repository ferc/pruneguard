<script context="module">
  export const metadata = { title: 'Counter Component' };
  export function getInitialCount() { return 0; }
</script>

<script>
  import { greet } from '../utils';

  let count = 0;
  let message = greet('Svelte');

  function increment() {
    count += 1;
  }
</script>

<div>
  <p>{message}</p>
  <button on:click={increment}>Count: {count}</button>
  {#if count > 5}
    <HighScoreBadge />
  {/if}
  {#each Array(count) as _, i}
    <CountDot index={i} />
  {/each}
</div>

<style>
  button {
    padding: 0.5rem 1rem;
  }
</style>
