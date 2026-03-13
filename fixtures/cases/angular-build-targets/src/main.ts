import { platformBrowserDynamic } from "@angular/platform-browser-dynamic";

platformBrowserDynamic()
  .bootstrapModule(undefined as any)
  .catch((err: unknown) => console.error(err));
