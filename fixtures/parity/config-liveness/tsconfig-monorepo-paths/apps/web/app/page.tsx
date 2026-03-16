import { Header } from '@/src/components/Header';
import { format } from '@mono/shared/utils';

export default function Page() {
  return <Header text={format('hello')} />;
}
