// Copies ../docs/**/*.md into Starlight's content folder so docs/ stays the single source.
//
// For each page:
//   - the first "# Heading" becomes the frontmatter title and is removed from the body;
//   - relative links to other docs pages become site URLs (with the base path and anchors);
//   - images inside docs/ are copied next to the pages and their relative links kept;
//   - relative links to repository files outside docs/ become GitHub URLs;
//   - an editUrl pointing at the original file in docs/ is added.
// docs/SUMMARY.md (the mdBook table of contents) is skipped; astro.config.mjs reads it for the sidebar.
//
// Run by `npm run dev` and `npm run build`. The output folder is gitignored, except index.mdx and demos/.

import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, posix, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { BASE, REPO_URL, docSlug } from '../site.config.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '../..');
const docsDir = join(repoRoot, 'docs');
const outDir = resolve(here, '../src/content/docs');
// Committed pages that live next to the synced ones (top-level names: a folder is kept whole).
const KEEP = new Set(['index.mdx', 'demos']);
const SKIP = new Set(['SUMMARY.md']);

function walk(dir) {
	return readdirSync(dir).flatMap((name) => {
		const full = join(dir, name);
		return statSync(full).isDirectory() ? walk(full) : [full];
	});
}

// Remove previously synced files (everything except KEEP), so deleted docs disappear.
function clean(dir, top = true) {
	if (!existsSync(dir)) return;
	for (const name of readdirSync(dir)) {
		const full = join(dir, name);
		if (top && KEEP.has(name)) continue;
		if (statSync(full).isDirectory()) {
			clean(full, false);
			if (readdirSync(full).length === 0) rmSync(full, { recursive: true });
		} else {
			rmSync(full);
		}
	}
}

const IMAGE = /\.(png|jpe?g|gif|webp|avif|svg)$/i;

const isExternal = (url) => /^[a-z][a-z0-9+.-]*:/i.test(url) || url.startsWith('//');

// docRel: path of the current page relative to docs/, e.g. "tools/leanqc.md".
function rewriteUrl(url, docRel) {
	if (!url || isExternal(url) || url.startsWith('#') || url.startsWith('/')) return url;
	const hashAt = url.indexOf('#');
	const pathPart = hashAt === -1 ? url : url.slice(0, hashAt);
	const hash = hashAt === -1 ? '' : url.slice(hashAt);
	if (!pathPart) return url;

	// Resolve against the page's location inside the repository.
	const repoRel = posix.normalize(posix.join('docs', posix.dirname(docRel), decodeURI(pathPart)));
	if (repoRel.startsWith('../')) {
		console.warn(`sync-docs: ${docRel}: link "${url}" leaves the repository; left as is`);
		return url;
	}

	// Images inside docs/ are copied next to the pages (below), so the relative link still works
	// and Astro optimises and serves them from the site.
	if (repoRel.startsWith('docs/') && IMAGE.test(repoRel)) {
		if (!existsSync(join(repoRoot, repoRel))) console.warn(`sync-docs: ${docRel}: link to missing image "${repoRel}"`);
		return url;
	}

	if (repoRel.startsWith('docs/') && repoRel.endsWith('.md')) {
		const target = repoRel.slice('docs/'.length);
		if (!existsSync(join(docsDir, target))) {
			// Keep it a site link so the links validator reports it.
			console.warn(`sync-docs: ${docRel}: link to missing page "${target}"`);
		}
		return `${BASE}/${docSlug(target)}/${hash}`;
	}

	// Anything else (LICENSE, crate sources, test data, images) is served from GitHub.
	const abs = join(repoRoot, repoRel);
	const kind = existsSync(abs) && statSync(abs).isDirectory() ? 'tree' : 'blob';
	if (!existsSync(abs)) console.warn(`sync-docs: ${docRel}: link to missing file "${repoRel}"`);
	return `${REPO_URL}/${kind}/main/${encodeURI(repoRel)}${hash}`;
}

// Rewrite inline links/images "](url ...)" and reference definitions "[id]: url",
// leaving fenced code blocks and inline code untouched.
function rewriteLinks(body, docRel) {
	const lines = body.split('\n');
	let fence = null;
	return lines
		.map((line) => {
			const fenceMatch = line.match(/^\s*(`{3,}|~{3,})/);
			if (fence) {
				if (fenceMatch && fenceMatch[1][0] === fence[0] && fenceMatch[1].length >= fence.length) fence = null;
				return line;
			}
			if (fenceMatch) {
				fence = fenceMatch[1];
				return line;
			}
			const def = line.match(/^(\s{0,3}\[[^\]]+\]:\s*)(<[^>]*>|\S+)(.*)$/);
			if (def) {
				const raw = def[2].startsWith('<') ? def[2].slice(1, -1) : def[2];
				return def[1] + rewriteUrl(raw, docRel) + def[3];
			}
			// Split out inline code spans so links inside them are not touched.
			return line
				.split(/(`+[^`]*`+)/)
				.map((part, i) =>
					i % 2 === 1
						? part
						: part.replace(/\]\(\s*(<[^>]*>|[^\s)]+)(\s+"[^"]*")?\s*\)/g, (_m, target, title = '') => {
								const raw = target.startsWith('<') ? target.slice(1, -1) : target;
								return `](${rewriteUrl(raw, docRel)}${title})`;
							})
				)
				.join('');
		})
		.join('\n');
}

function convert(source, docRel) {
	const text = source.replace(/\r\n/g, '\n');
	const lines = text.split('\n');
	let title = null;
	let fence = null;
	for (let i = 0; i < lines.length; i++) {
		const f = lines[i].match(/^\s*(`{3,}|~{3,})/);
		if (f) fence = fence ? null : f[1];
		if (fence) continue;
		const h = lines[i].match(/^#\s+(.+?)\s*#*\s*$/);
		if (h) {
			title = h[1];
			lines.splice(i, 1);
			if (lines[i] === '') lines.splice(i, 1);
			break;
		}
	}
	if (!title) {
		title = posix.basename(docRel, '.md');
		console.warn(`sync-docs: ${docRel}: no "# Heading"; using "${title}" as the title`);
	}
	const frontmatter = [
		'---',
		`title: ${JSON.stringify(title)}`,
		`editUrl: ${JSON.stringify(`${REPO_URL}/edit/main/docs/${docRel}`)}`,
		'---',
		'',
		`<!-- Generated from docs/${docRel} by site/scripts/sync-docs.mjs. Edit the original. -->`,
	];
	return `${frontmatter.join('\n')}\n\n${rewriteLinks(lines.join('\n'), docRel)}`;
}

clean(outDir);
let count = 0;
for (const file of walk(docsDir)) {
	const docRel = relative(docsDir, file).split('\\').join('/');
	if (IMAGE.test(docRel)) {
		const dest = join(outDir, docRel);
		mkdirSync(dirname(dest), { recursive: true });
		copyFileSync(file, dest);
		continue;
	}
	if (!docRel.endsWith('.md') || SKIP.has(docRel)) continue;
	const dest = join(outDir, docRel);
	mkdirSync(dirname(dest), { recursive: true });
	writeFileSync(dest, convert(readFileSync(file, 'utf8'), docRel));
	count++;
}
console.log(`sync-docs: ${count} pages from docs/ -> site/src/content/docs/`);
