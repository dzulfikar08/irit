// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import starlightLinksValidator from 'starlight-links-validator';
import { BASE, REPO_URL, SITE, sidebarFromSummary } from './site.config.mjs';

export default defineConfig({
	site: SITE,
	base: BASE,
	trailingSlash: 'always',
	integrations: [
		starlight({
			title: 'irit',
			description:
				'Small, fast command-line tools for the manual steps around sequencing pipelines, built to run on ordinary laptops.',
			logo: {
				light: './src/assets/mark-light.svg',
				dark: './src/assets/mark-dark.svg',
				alt: '',
			},
			favicon: '/favicon.svg',
			social: [{ icon: 'github', label: 'Source on GitHub', href: REPO_URL }],
			// Pages carry their own editUrl (set by scripts/sync-docs.mjs) pointing at docs/.
			sidebar: sidebarFromSummary(),
			customCss: [
				'@fontsource-variable/atkinson-hyperlegible-next/wght.css',
				'@fontsource-variable/atkinson-hyperlegible-next/wght-italic.css',
				'@fontsource-variable/atkinson-hyperlegible-mono/wght.css',
				'./src/styles/theme.css',
			],
			expressiveCode: {
				themes: ['github-light', 'github-dark-dimmed'],
				// Plain code frames for shell snippets too: no imitation terminal title bar.
				defaultProps: { wrap: true, overridesByLang: { 'bash,sh,shell,zsh': { frame: 'code' } } },
				styleOverrides: {
					borderRadius: '4px',
					codeFontFamily: "'Atkinson Hyperlegible Mono Variable', ui-monospace, monospace",
					uiFontFamily: "'Atkinson Hyperlegible Next Variable', system-ui, sans-serif",
					frames: { shadowColor: 'transparent' },
				},
			},
			plugins: [starlightLinksValidator({ errorOnFallbackPages: false })],
		}),
	],
});
