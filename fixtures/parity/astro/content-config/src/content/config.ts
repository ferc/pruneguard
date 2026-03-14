import { defineCollection } from 'astro:content';
import { blogSchema } from '../schemas/blog';

const blog = defineCollection({ schema: blogSchema });

export const collections = { blog };
