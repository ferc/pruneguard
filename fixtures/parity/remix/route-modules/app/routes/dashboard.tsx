import { getSession } from '../utils/session';

export async function loader({ request }: { request: Request }) {
  const session = await getSession(request);
  return { user: session.user };
}

export default function Dashboard() {
  return '<h1>Dashboard</h1>';
}
