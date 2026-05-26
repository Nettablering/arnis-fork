// @ts-check
import { defineConfig } from "astro/config";
import mdx from "@astrojs/mdx";
import tailwindcss from "@tailwindcss/vite";

// Interim canonical per Q503; flips to https://worldbuilders.app at T-0.
const SITE = process.env.WB_SITE_URL ?? "https://worldbuilders.quicktoolry.com";

export default defineConfig({
  site: SITE,
  output: "static",
  integrations: [mdx()],
  vite: {
    plugins: [tailwindcss()],
  },
  build: {
    assets: "assets",
  },
  server: {
    host: "127.0.0.1",
    port: 4321,
  },
});
