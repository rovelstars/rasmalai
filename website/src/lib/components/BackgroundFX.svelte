<script lang="ts">
	// Ambient page atmosphere: static film grain and two slow-drifting aura
	// orbs. All layers are fixed, pointer-transparent, transform/opacity-only,
	// and frozen under prefers-reduced-motion.
</script>

<div class="fx" aria-hidden="true">
	<div class="fx-orb fx-orb-a"></div>
	<div class="fx-orb fx-orb-b"></div>
	<svg class="fx-grain" width="100%" height="100%">
		<filter id="rnx-noise">
			<feTurbulence type="fractalNoise" baseFrequency="0.8" numOctaves="3" stitchTiles="stitch" />
		</filter>
		<rect width="100%" height="100%" filter="url(#rnx-noise)" />
	</svg>
</div>

<style>
	.fx {
		position: fixed;
		inset: 0;
		z-index: 0;
		pointer-events: none;
		overflow: hidden;
	}
	.fx-orb {
		position: absolute;
		border-radius: 9999px;
		filter: blur(140px);
		will-change: transform, opacity;
	}
	.fx-orb-a {
		width: 560px;
		height: 560px;
		left: 8%;
		top: -180px;
		background: rgba(162, 119, 255, 0.1);
		animation: drift-a 16s ease-in-out infinite alternate;
	}
	.fx-orb-b {
		width: 480px;
		height: 480px;
		right: 4%;
		top: -120px;
		background: rgba(130, 226, 255, 0.07);
		animation: drift-b 16s ease-in-out infinite alternate-reverse;
	}
	@keyframes drift-a {
		from {
			transform: translate(0, 0) scale(1);
			opacity: 0.8;
		}
		to {
			transform: translate(90px, 60px) scale(1.12);
			opacity: 1;
		}
	}
	@keyframes drift-b {
		from {
			transform: translate(0, 0) scale(1.05);
			opacity: 1;
		}
		to {
			transform: translate(-70px, 50px) scale(0.95);
			opacity: 0.75;
		}
	}
	.fx-grain {
		position: absolute;
		inset: 0;
		opacity: 0.035;
	}
	@media (prefers-reduced-motion: reduce) {
		.fx-orb-a,
		.fx-orb-b {
			animation: none;
		}
	}
	:global(html.light) .fx-orb-a {
		background: rgba(109, 63, 212, 0.08);
	}
	:global(html.light) .fx-orb-b {
		background: rgba(0, 109, 158, 0.06);
	}
</style>
