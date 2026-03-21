<script lang="ts">
  let { children } = $props();
</script>

{@render children()}

<style>
  :global(*, *::before, *::after) { box-sizing: border-box; margin: 0; padding: 0; }

  :global(body) {
    background: transparent;
    color: var(--text, #e2e8f0);
    font-family: 'Hiragino Sans', 'Yu Gothic UI', 'Meiryo UI', 'Segoe UI', system-ui, sans-serif;
    font-size: var(--font-size, 13px);
    overflow: hidden;
  }

  /* ── Default (= dark) ── */
  :global(:root) {
    --bg:         13 13 23;
    --surface:    22 22 40;
    --border:     #2d2d4e;
    --text:       #e2e8f0;
    --text-dim:   #4a5568;
    --accent:     #818cf8;
    --accent-glow:rgba(129,140,248,0.35);
    --input-bg:   rgba(255 255 255 / 0.05);
    --thumb:      #818cf8;
    /* channel colors — game-accurate */
    --c-world:    #a78bfa;   /* purple */
    --c-guild:    #34d399;   /* green  */
    --c-party:    #60a5fa;   /* blue   */
    --c-channel:  #e2e8f0;   /* white  */
    --bg-opacity: 0.92;
    --font-size:  13px;
  }

  :global([data-theme="dark"]) {
    --bg:         13 13 23;
    --surface:    22 22 40;
    --border:     #2d2d4e;
    --text:       #e2e8f0;
    --text-dim:   #4a5568;
    --accent:     #818cf8;
    --accent-glow:rgba(129,140,248,0.35);
    --input-bg:   rgba(255 255 255 / 0.05);
    --thumb:      #818cf8;
    --c-world:    #a78bfa;
    --c-guild:    #34d399;
    --c-party:    #60a5fa;
    --c-channel:  #e2e8f0;
  }

  :global([data-theme="light"]) {
    --bg:         248 250 252;
    --surface:    241 245 249;
    --border:     #cbd5e1;
    --text:       #1e293b;
    --text-dim:   #94a3b8;
    --accent:     #6d28d9;
    --accent-glow:rgba(109,40,217,0.2);
    --input-bg:   rgba(0 0 0 / 0.05);
    --thumb:      #6d28d9;
    /* light bg → use darker shades for contrast */
    --c-world:    #7c3aed;
    --c-guild:    #16a34a;
    --c-party:    #2563eb;
    --c-channel:  #475569;   /* dark gray (white invisible on light bg) */
  }

  :global([data-theme="oled"]) {
    --bg:         0 0 0;
    --surface:    8 8 8;
    --border:     #1a1a1a;
    --text:       #ffffff;
    --text-dim:   #3a3a3a;
    --accent:     #d8b4fe;
    --accent-glow:rgba(216,180,254,0.3);
    --input-bg:   rgba(255 255 255 / 0.04);
    --thumb:      #d8b4fe;
    --c-world:    #d8b4fe;
    --c-guild:    #6ee7b7;
    --c-party:    #93c5fd;
    --c-channel:  #f8fafc;
  }

  :global([data-theme="cyber"]) {
    --bg:         0 5 15;
    --surface:    0 12 28;
    --border:     #003322;
    --text:       #00ff88;
    --text-dim:   #004422;
    --accent:     #00ff88;
    --accent-glow:rgba(0,255,136,0.4);
    --input-bg:   rgba(0 255 136 / 0.04);
    --thumb:      #00ff88;
    /* on neon-green bg — use vivid distinct hues */
    --c-world:    #e879f9;   /* magenta-purple */
    --c-guild:    #4ade80;   /* bright green   */
    --c-party:    #38bdf8;   /* sky blue       */
    --c-channel:  #e0f2fe;   /* near-white     */
  }

  :global([data-theme="sakura"]) {
    --bg:         18 5 12;
    --surface:    30 8 20;
    --border:     #4a1535;
    --text:       #fce7f3;
    --text-dim:   #6b1a40;
    --accent:     #f472b6;
    --accent-glow:rgba(244,114,182,0.35);
    --input-bg:   rgba(255 255 255 / 0.04);
    --thumb:      #f472b6;
    --c-world:    #c084fc;
    --c-guild:    #4ade80;
    --c-party:    #60a5fa;
    --c-channel:  #fce7f3;
  }

  :global([data-theme="midnight"]) {
    --bg:         5 8 25;
    --surface:    10 15 45;
    --border:     #1e2a5e;
    --text:       #e0e7ff;
    --text-dim:   #2d3a6e;
    --accent:     #c4b5fd;
    --accent-glow:rgba(196,181,253,0.35);
    --input-bg:   rgba(255 255 255 / 0.04);
    --thumb:      #c4b5fd;
    --c-world:    #c4b5fd;
    --c-guild:    #6ee7b7;
    --c-party:    #93c5fd;
    --c-channel:  #e0e7ff;
  }

  :global([data-theme="nord"]) {
    --bg:         36 41 56;
    --surface:    46 52 72;
    --border:     #434c6e;
    --text:       #eceff4;
    --text-dim:   #4c566a;
    --accent:     #b48ead;
    --accent-glow:rgba(180,142,173,0.3);
    --input-bg:   rgba(255 255 255 / 0.05);
    --thumb:      #b48ead;
    --c-world:    #b48ead;   /* nord purple */
    --c-guild:    #a3be8c;   /* nord green  */
    --c-party:    #81a1c1;   /* nord blue   */
    --c-channel:  #eceff4;   /* nord snow   */
  }
</style>
