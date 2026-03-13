const modules = import.meta.glob(['./a/*.ts', './b/*.ts']);
export { modules };
