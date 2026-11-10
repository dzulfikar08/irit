// Shared by astro.config.mjs and scripts/sync-docs.mjs.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

export const SITE = 'https://dzulfikar08.github.io';
export const BASE = '/irit';
export const REPO_URL = 'https://github.com/dzulfikar08/irit';

const docsDir = fileURLToPath(new URL('../docs/', import.meta.url));

// Same rule as Astro's glob loader (github-slugger on each path segment, extension dropped).
const slugSegment = (s) =>
	s
		.toLowerCase()
		.replace(/[^\p{L}\p{M}\p{N}\p{Pc} -]/gu, '')
		.replace(/ /g, '-');
export const docSlug = (docRel) => docRel.replace(/\.md$/, '').split('/').map(slugSegment).join('/');

function listDocs(dir = docsDir) {
	return readdirSync(dir).flatMap((name) => {
		const full = join(dir, name);
		if (statSync(full).isDirectory()) return listDocs(full);
		const rel = relative(docsDir, full).split('\\').join('/');
		return rel.endsWith('.md') && rel !== 'SUMMARY.md' ? [rel] : [];
	});
}

const folderOf = (rel) => (rel.includes('/') ? rel.slice(0, rel.lastIndexOf('/')) : '');

/**
 * Sidebar derived from docs/SUMMARY.md (mdBook format):
 *   [Title](page.md)        -> top-level link before the first "# Part"
 *   # Part                  -> group
 *   - [Label](dir/page.md)  -> link in that group
 * Pages in docs/ that SUMMARY.md doesn't list are still shown: in the group whose listed pages
 * all live in the same folder (so docs/tools/msascope.md joins "Tools"), otherwise in "More".
 */
export function sidebarFromSummary() {
	const summary = readFileSync(join(docsDir, 'SUMMARY.md'), 'utf8');
	const top = [];
	const groups = [];
	const listed = new Set();
	for (const line of summary.split('\n')) {
		const part = line.match(/^#\s+(.+?)\s*$/);
		if (part) {
			if (part[1].toLowerCase() !== 'summary') groups.push({ label: part[1], items: [], files: [] });
			continue;
		}
		const link = line.match(/^\s*(?:[-*]\s+)?\[([^\]]+)\]\(([^)#\s]+\.md)\)/);
		if (!link) continue;
		const [, label, file] = link;
		const rel = file.replace(/^\.\//, '');
		listed.add(rel);
		const item = { label, slug: docSlug(rel) };
		if (groups.length) {
			groups.at(-1).items.push(item);
			groups.at(-1).files.push(rel);
		} else {
			top.push(item);
		}
	}

	const orphans = listDocs().filter((rel) => !listed.has(rel)).sort();
	const more = [];
	for (const rel of orphans) {
		const home = groups.find((g) => {
			const folders = new Set(g.files.map(folderOf));
			return folders.size === 1 && folders.has(folderOf(rel)) && folderOf(rel) !== '';
		});
		// No label: Starlight uses the page title.
		(home ? home.items : more).push({ slug: docSlug(rel) });
	}
	if (more.length) groups.push({ label: 'More', items: more });

	return [...top, ...groups.map(({ label, items }) => ({ label, items }))];
}
