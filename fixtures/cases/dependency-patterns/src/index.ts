import { helper } from './helper';

const configPath = require.resolve('./config');
const modules = import.meta.glob('./modules/*.ts');

export { helper, configPath, modules };
