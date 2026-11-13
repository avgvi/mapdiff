// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// The docs site, served by GitHub Pages at https://avgvi.github.io/mapdiff/.
// Logos and screenshots are read from ../docs, so the README and the site
// show the same files.
export default defineConfig({
	site: 'https://avgvi.github.io',
	base: '/mapdiff',
	vite: { server: { fs: { allow: ['..'] } } },
	integrations: [
		starlight({
			title: 'mapdiff',
			description:
				'Review a change on a map of your code: diffs by function and type, drawn with what they touch.',
			logo: {
				dark: '../docs/brand/logo-dark.svg',
				light: '../docs/brand/logo-light.svg',
				replacesTitle: true,
			},
			favicon: '/favicon.svg',
			customCss: ['@fontsource-variable/space-grotesk', './src/styles/mapdiff.css'],
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/avgvi/mapdiff' }],
			editLink: { baseUrl: 'https://github.com/avgvi/mapdiff/edit/main/website/' },
			sidebar: [
				{
					label: 'Start here',
					items: [
						{ label: 'Install', slug: 'getting-started/install' },
						{ label: 'Quick start', slug: 'getting-started/quick-start' },
					],
				},
				{
					label: 'Guides',
					items: [
						{ label: 'Reviewing a change', slug: 'guides/review' },
						{ label: 'The map', slug: 'guides/map' },
						{ label: 'Keys', slug: 'guides/keys' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'Commands', slug: 'reference/cli' },
						{ label: 'Languages and references', slug: 'reference/languages' },
						{ label: 'Changelog', slug: 'reference/changelog' },
					],
				},
			],
		}),
	],
});
