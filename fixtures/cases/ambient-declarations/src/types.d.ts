// Ambient declaration file — should NOT be flagged as unused.
interface AppConfig {
  debug: boolean;
}

declare module "*.css" {
  const styles: Record<string, string>;
  export default styles;
}
