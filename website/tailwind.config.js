/** @type {import('tailwindcss').Config} */
const v = (name) => `rgb(var(--aura-${name}) / <alpha-value>)`;

export default {
	darkMode: 'class',
	content: ['./src/**/*.{html,js,svelte,ts}'],
	theme: {
		extend: {
			colors: {
				aura: {
					bg: v('bg'),
					surface: v('surface'),
					surfaceElevated: v('surfaceElevated'),
					border: v('line'),
					borderHover: v('lineStrong'),
					purple: v('purple'),
					green: v('green'),
					orange: v('orange'),
					pink: v('pink'),
					cyan: v('cyan'),
					red: v('red'),
					text: v('text'),
					muted: v('muted'),
					faint: v('faint'),
					signal: v('green'),
					link: v('cyan')
				}
			},
			fontFamily: {
				sans: ['Inter', 'system-ui', 'sans-serif'],
				mono: ['JetBrains Mono', 'ui-monospace', 'monospace']
			},
			maxWidth: {
				page: '1440px'
			},
			borderRadius: {
				none: '0',
				sm: 'var(--radius-sm)',
				DEFAULT: 'var(--radius-sm)',
				md: 'var(--radius-md)',
				lg: 'var(--radius-md)',
				xl: 'var(--radius-md)',
				full: 'var(--radius-pill)'
			}
		}
	},
	plugins: [require('@tailwindcss/typography')]
};
