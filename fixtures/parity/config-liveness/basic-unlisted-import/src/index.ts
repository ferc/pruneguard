import express from 'express';
import chalk from 'chalk'; // chalk is NOT declared in package.json

const app = express();
console.log(chalk.green('Server started'));
export default app;
