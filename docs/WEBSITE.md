# Website and custom domain

The presentation website lives in `site/`. It is static HTML, CSS and a small script for Spanish, English, Simplified Chinese and Japanese; it has no build step, backend, analytics or domain-specific URLs. The small image copies in `site/assets/` make the folder self-contained for publishing; refresh them from `docs/images/` if the screenshots change.

The Chinese and Japanese translations apply to this presentation site. The application interface itself is currently translated into Spanish and English.

## Preview locally

Open `site/index.html` in a browser. To preview the same relative paths through a local server, run `python -m http.server 8000 --directory .` from the repository root and open `http://localhost:8000/site/`.

## When the domain is ready

1. Publish the repository on GitHub and set GitHub Pages to use GitHub Actions. Add a deployment workflow that uploads `site/` as the Pages artifact and deploys it; the existing `Build` workflow only compiles the application.
2. In the repository's Pages settings, enter the purchased hostname and enable HTTPS once GitHub has verified the DNS records.
3. Add the DNS records GitHub Pages shows for that hostname at the domain registrar. Follow the current values and instructions shown in GitHub Pages; do not copy example IP addresses from old notes.
4. Verify both the apex hostname and `www` if both will be used, then choose one canonical address and redirect the other to it.
5. Keep the site links relative for its own assets so the same files work on a preview URL and on the custom domain.

The domain is intentionally not written into the site yet. The current Actions build workflow compiles the application; it does not deploy the presentation site.
