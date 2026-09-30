# Hosted downloads

The downloads site remains live at:

```text
https://averagechris.srht.site/slack/
```

It contains the historical SourceHut releases, including their original Linux
artifacts, checksums, manifests, and stable download URLs. Preserve those
records so existing links continue to work.

Future releases are GitHub-only. The SourceHut build manifest and local Pages
applications are archival implementation history: do not submit the manifest,
upload new artifacts through SourceHut, or republish the site with those local
tools. The downloads site is refreshed from the verified GitHub Release by the
separate Pages workflow.

For the current release commands, six-to-four artifact verification, safe
draft publication, and the exact Pages dispatch, follow
[`docs/release.md`](release.md).

Historical tarballs remain suitable for manual installation after checking the
adjacent SHA-256 sidecar. For example, retained records use URLs shaped like:

```text
https://averagechris.srht.site/slack/downloads/slack-v0.3.1-x86_64-linux.tar.gz
https://averagechris.srht.site/slack/downloads/slack-v0.3.1-x86_64-linux.tar.gz.sha256
```
