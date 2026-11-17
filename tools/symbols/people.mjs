// Draws the people sets (C21 Open Peeps, C22 Notionists, C23 Micah) with
// DiceBear, a fixed list of seeds so every build gives the same figures.
//   node tools/symbols/people.mjs <node_modules with @dicebear/core@9 and @dicebear/collection@9> <out.json>
import { createRequire } from "node:module";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

const [modules, out] = process.argv.slice(2);
const require = createRequire(join(modules, "x.js"));
const { createAvatar } = require("@dicebear/core");
const collection = require("@dicebear/collection");

const COUNT = 120;
const SETS = [
  { id: "C21", key: "open-peeps", style: "openPeeps", name: "Open Peeps", author: "Pablo Stanley", author_url: "https://www.openpeeps.com", licence: "CC0 1.0", licence_spdx: "CC0-1.0", licence_url: "https://creativecommons.org/publicdomain/zero/1.0/", credit: false },
  { id: "C22", key: "notionists", style: "notionists", name: "Notionists", author: "Zoish", author_url: "https://heyzoish.gumroad.com/l/notionists", licence: "CC0 1.0", licence_spdx: "CC0-1.0", licence_url: "https://creativecommons.org/publicdomain/zero/1.0/", credit: false },
  { id: "C23", key: "micah", style: "micah", name: "Micah", author: "Micah Lanier", author_url: "https://dribbble.com/micahlanier", licence: "CC BY 4.0", licence_spdx: "CC-BY-4.0", licence_url: "https://creativecommons.org/licenses/by/4.0/", credit: true },
];

const sets = SETS.map(({ style, ...meta }) => {
  const items = [];
  for (let i = 1; i <= COUNT; i++) {
    const svg = createAvatar(collection[style], { seed: `zebra-${i}` }).toString();
    items.push({ name: `${meta.name} ${i}`, keywords: ["person", "people", "face", "avatar"], svg });
  }
  return { ...meta, group: "people", tint: "", items };
});
writeFileSync(out, JSON.stringify(sets));
console.error(sets.map((s) => `${s.id} ${s.items.length}`).join("\n"));
