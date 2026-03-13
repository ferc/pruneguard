<script context="module">
  export const metadata = { title: 'Svelte Component' };
</script>

<script>
  import { greet } from '../utils';

  let message = greet('Svelte');
</script>

<p>{message}</p>
