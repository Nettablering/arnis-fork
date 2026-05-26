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

  const updated = posts[0]?.data.date.toISOString() ?? new Date().toISOString();

  const entries = posts.map((p) => {
    const url = `${base}/blog/${p.slug}/`;
    return `  <entry>
    <title>${escapeXml(p.data.title)}</title>
    <link href="${url}" />
    <id>${url}</id>
    <updated>${p.data.date.toISOString()}</updated>
    <published>${p.data.date.toISOString()}</published>
    <summary>${escapeXml(p.data.summary)}</summary>
    <author><name>${escapeXml(p.data.author)}</name></author>
    ${p.data.tags.map((t) => `<category term="${escapeXml(t)}" />`).join("\n    ")}
  </entry>`;
  }).join("\n");

  const xml = `<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Worldbuilders devblog</title>
  <link href="${base}/blog/atom.xml" rel="self" />
  <link href="${base}/blog/" />
  <id>${base}/blog/</id>
  <updated>${updated}</updated>
  <subtitle>Weekly build-in-public devblog from Worldbuilders.</subtitle>
${entries}
</feed>`;

  return new Response(xml, {
    headers: { "Content-Type": "application/atom+xml; charset=utf-8" },
  });
};
