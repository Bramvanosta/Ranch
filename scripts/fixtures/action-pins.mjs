// The actions the workflows may use, each pinned to the commit of an exact
// release (resolved read-only: gh api repos/<o>/<r>/git/ref/tags/<tag>, an
// annotated tag dereferenced). workflows.test.mjs holds every workflow to it.
// To bump an action: resolve the new tag's commit, change it here and in the
// workflows, in one commit.
export const PINS = {
  "actions/checkout": ["fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09", "v5.1.0"],
  "actions/setup-node": ["a0853c24544627f65ddf259abe73b1d18a591444", "v5.0.0"],
  "actions/cache": ["0057852bfaa89a56745cba8c7296529d2fc39830", "v4.3.0"],
  "actions/upload-artifact": ["b7c566a772e6b6bfb58ed0dc250532a479d7789f", "v6.0.0"],
  "actions/download-artifact": ["018cc2cf5baa6db3ef3c5f8a56943fffe632ef53", "v6.0.0"],
  "docker/setup-buildx-action": ["8d2750c68a42422c14e847fe6c8ac0403b4cbd6f", "v3.12.0"],
  "docker/build-push-action": ["10e90e3645eae34f1e60eeb005ba3a3d33f178e8", "v6.19.2"],
  "docker/login-action": ["c94ce9fb468520275223c153574b00df6fe4bcc9", "v3.7.0"],
  "Swatinem/rust-cache": ["d1527c8f67ef9af357290475d4af4b0fab8f12e0", "v2.9.3"],
}
