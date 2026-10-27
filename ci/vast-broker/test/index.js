// Keep the owner's exact directory command portable, and include every probe.
// An EXPECTED RED transition must remain visible until its implementation lands.
import { readdir } from 'node:fs/promises';
for (const name of (await readdir(new URL('./', import.meta.url))).sort()) {
  if (name.endsWith('.test.js')) await import(new URL(name, import.meta.url));
}
