/** @type {import('tailwindcss').Config} */
export default {
	darkMode: 'class',
	content: ['./src/**/*.{html,js,svelte,ts}'],
	theme: {
		extend: {
			colors: {
				aura: {
					bg: '#15141b',
					surface: '#1c1b22',
					surfaceElevated: '#23222b',
					border: 'rgba(255, 255, 255, 0.08)',
					borderHover: 'rgba(162, 119, 255, 0.3)',
					purple: '#a277ff',
					green: '#61ffca',
					orange: '#ffca85',
					pink: '#f694ff',
					cyan: '#82e2ff',
					red: '#ff6767',
					text: '#edecee',
					muted: '#6d6d6d'
				}
			},
			fontFamily: {
				sans: ['Inter', 'system-ui', 'sans-serif'],
				mono: ['JetBrains Mono', 'Menlo', 'monospace']
			}
		}
	},
	plugins: [require('@tailwindcss/typography')]
};
