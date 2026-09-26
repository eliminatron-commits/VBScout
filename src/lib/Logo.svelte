<script lang="ts">
  // Same artwork as src-tauri/icons/source.svg.
  let { size = 32 }: { size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 1024 1024" aria-hidden="true" focusable="false">
  <defs>
    <linearGradient id="vbs-logo-bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#312e81" />
      <stop offset="1" stop-color="#0e7490" />
    </linearGradient>
  </defs>
  <rect x="64" y="64" width="896" height="896" rx="200" fill="url(#vbs-logo-bg)" />
  <rect x="200" y="262" width="400" height="56" rx="28" fill="#ffffff" />
  <rect x="200" y="378" width="300" height="56" rx="28" fill="#ffffff" fill-opacity="0.85" />
  <rect x="200" y="494" width="220" height="56" rx="28" fill="#ffffff" fill-opacity="0.7" />
  <circle cx="606" cy="586" r="172" fill="#ffffff" fill-opacity="0.14" stroke="#fbbf24" stroke-width="64" />
  <line x1="734" y1="714" x2="842" y2="822" stroke="#fbbf24" stroke-width="84" stroke-linecap="round" />
</svg>
