const modules = import.meta.glob(['./modules/*.ts', '!./modules/excluded.ts']);
export { modules };
