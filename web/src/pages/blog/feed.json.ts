import type { APIRoute } from "astro";
import { getCollection } from "astro:content";

export const GET: APIRoute = async ({ site }) => {
  const base = (site ?? new URL("https://worldbuilders.quicktoolry.com")).toString().replace(/\/$/, "");
  const posts = (await getCollection("blog", ({ data }) => !data.draft))
    .sort((a, b) => b.data.date.getTime() - a.data.date.getTime());

  // JSON Feed 1.1 — https://www.jsonfeed.org/version/1.1/
  const feed = {
    version: "https://jsonfeed.org/version/1.1",
    title: "Worldbuilders devblog",
    home_page_url: `${base}/blog/`,
    feed_url: `${base}/blog/feed.json`,
    description: "Weekly build-in-public devblog from Worldbuilders — turning OpenStreetMap into a Roblox idle empire.",
    language: "en",
    authors: [{ name: "Worldbuilders / Klokk Nettablering" }],
    items: posts.map((p) => ({
      id: `${base}/blog/${p.slug}/`,
      url: `${base}/blog/${p.slug}/`,
      title: p.data.title,
      summary: p.data.summary,
      content_text: p.data.summary,
      date_published: p.data.date.toISOString(),
      tags: p.data.tags,
      authors: [{ name: p.data.author }],
    })),
  };

  return new Response(JSON.stringify(feed, null, 2), {
    headers: { "Content-Type": "application/feed+json; charset=utf-8" },
  });
};
