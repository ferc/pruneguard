import { z } from 'astro:content';

export const blogSchema = z.object({
  title: z.string(),
  date: z.date(),
  draft: z.boolean().default(false),
});
