import type { APIRoute } from "astro";
import { getCollection } from "astro:content";

const escapeXml = (s: string) =>
  s.replace(/[<>&'"]/g, (c) =>
    ({ "<": "&lt;", ">": "&gt;", "&": "&amp;", "'": "&apos;", '"': "&quot;" }[c]!),
  );

export const GET: APIRoute = async ({ site }) => {
  const base = (site ?? new URL("https://worldbuilders.quicktoolry.com")).toString().replace(/\/$/, "");
  const posts = (await getCollection("blog", ({ data }) => !data.draft))
    .sort((a, b) => b.data.date.getTime() - a.data.date.getTime());

  const items = posts.map((p) => {
    const url = `${base}/blog/${p.slug}/`;
    return `    <item>
      <title>${escapeXml(p.data.title)}</title>
      <link>${url}</link>
      <guid isPermaLink="true">${url}</guid>
      <pubDate>${p.data.date.toUTCString()}</pubDate>
      <description>${escapeXml(p.data.summary)}</description>
      <author>noreply@worldbuilders.app (${escapeXml(p.data.author)})</author>
      ${p.data.tags.map((t) => `<category>${escapeXml(t)}</category>`).join("\n      ")}
    </item>`;
  }).join("\n");

  const xml = `<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom">
  <channel>
    <title>Worldbuilders devblog</title>
    <link>${base}/blog/</link>
    <atom:link href="${base}/blog/rss.xml" rel="self" type="application/rss+xml" />
    <description>Weekly build-in-public devblog from Worldbuilders — turning OpenStreetMap into a Roblox idle empire.</description>
    <language>en</language>
    <lastBuildDate>${new Date().toUTCString()}</lastBuildDate>
${items}
  </channel>
</rss>`;

  return new Response(xml, {
    headers: { "Content-Type": "application/rss+xml; charset=utf-8" },
  });
};
