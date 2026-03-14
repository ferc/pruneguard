import { Layout } from './components/Layout';

export default function App() {
  return Layout({ children: '<slot />' });
}

export function links() {
  return [{ rel: 'stylesheet', href: '/styles.css' }];
}
