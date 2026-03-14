import { fetchUser } from '@API/endpoint';
import { colors } from '@DS/tokens';
import { Button } from '@Components/Button';

export function render() {
  return { user: fetchUser(), colors, Button };
}
