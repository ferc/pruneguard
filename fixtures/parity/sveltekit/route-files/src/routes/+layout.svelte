<script>
  import { formatDate } from '$lib/utils';
</script>

<nav>SvelteKit App</nav>
<slot />
<footer>{formatDate(new Date())}</footer>
