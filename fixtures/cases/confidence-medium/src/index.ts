import { loadPlugins } from "plugin-system";

// Dynamic plugin loading means some imports may be unresolvable
const plugins = loadPlugins("./plugins");

console.log(plugins);
