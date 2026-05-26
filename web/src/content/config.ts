import { defineCollection, z } from "astro:content";

// Q511 — devblog content collection. Frontmatter schema is Zod-validated at
// build; see /docs/grill/q511-website-blog-devblog-cadence.md and
// /docs/devblog-cadence.md for the editorial contract.
const blog = defineCollection({
  type: "content",
  schema: z.object({
    title: z.string().min(8).max(140),
    date: z.coerce.date(),
    tags: z.array(z.string()).default([]),
    summary: z.string().min(20).max(400),
    author: z.string().default("rolf"),
    heroImageAlt: z.string().optional(),
    heroImage: z.string().optional(),
    draft: z.boolean().default(false),
    // Per the design system, blog routes default to paper (light) but can
    // opt back into ink (dark) per post.
    theme: z.enum(["ink", "paper"]).default("paper"),
    // Cartographic latitude marker for the publication date — rendered in
    // the article header as a mono coordinate readout.
    latMarker: z.string().default("LAT 62°28′ N"),
  }),
});

export const collections = { blog };
