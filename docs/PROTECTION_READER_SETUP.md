# Protection reader setup

**Activation is deferred to a follow-up at the user's request.** The repository
contains prepared configuration and fail-closed integration only. No App has been
registered, installed, or configured by this change; publication remains inactive
until the owner completes that follow-up.

Stable and rolling development publication must read the current classic `main`
protection, including administrator enforcement and strict required checks. The
normal `GITHUB_TOKEN` has no Administration permission. The REST protection endpoint
requires Administration read; using GraphQL does not expose these missing settings
to a repository reader. See [GitHub's protection API](https://docs.github.com/en/rest/branches/branch-protection#get-branch-protection)
and the [Scorecard GraphQL implementation](https://github.com/ossf/scorecard/blob/main/clients/githubrepo/branches.go).

The reviewed configuration is [protection-app-manifest.json](protection-app-manifest.json).
It contains no credential. A dedicated private GitHub App needs Administration
read and automatic Metadata read, installed only on `ropbet-radbyt/sorotte`.
It needs no contents, Actions, checks, packages, organization, or write permission.
The manifest records the registration configuration; it is not a credential or an
automatic installation. GitHub's [manifest flow](https://docs.github.com/en/apps/sharing-github-apps/registering-a-github-app-from-a-manifest)
requires an owner-controlled registration callback and code exchange. Manual
registration below avoids adding a callback service for this one installation.

An owner can complete the setup as follows:

1. In **Settings → Developer settings → GitHub Apps → New GitHub App**, register a
   private app using the name, homepage, description, and permissions from the
   manifest. Use a unique name if the suggested one is already taken. Disable
   webhooks and user OAuth authorization; subscribe to no events.
2. Install it on **Only select repositories → sorotte**. Verify Administration is
   **Read-only** and no write permissions appear in the installation review.
3. Generate one private key in the App settings. Add the PEM directly as the
   repository Actions secret `SOROTTE_PROTECTION_APP_PRIVATE_KEY`. Store the App ID
   as the repository Actions variable `SOROTTE_PROTECTION_APP_ID`. The ID is not
   an installation ID or client ID. Keep the private key out of this checkout,
   artifacts, logs, chat, and shell history.
4. On an exact current `main` SHA whose required checks pass, run the coordinated
   stable-release workflow with `publish=false` after arranging its documented
   ephemeral native runner. Its first authorization step proves the App can read
   the full protection and binds the successful required checks before any build
   qualification. Missing or insufficient App access fails closed.

The pinned `actions/create-github-app-token` action requests a token scoped to the
current repository with `permission-administration: read`, immediately before each
protection authorization. It revokes the token when the job ends. The App token is
passed only as `SOROTTE_PROTECTION_TOKEN` to the protection REST lookup; ordinary
checks and Actions queries retain `GH_TOKEN: github.token`. The development job
waits for normal checks before minting, so waiting cannot consume the token's
one-hour lifetime. These inputs and revocation default were verified against the
[pinned action definition](https://github.com/actions/create-github-app-token/blob/fee1f7d63c2ff003460e3d139729b119787bc349/action.yml).

The private key is forwarded by name only to publication workflows. Package CI,
pull-request CI, and native candidate jobs receive no App credential. Rotating the
key means adding the replacement secret, verifying a read authorization, then
revoking the old key in App settings. Removing the App or either configuration
value disables publication without bypassing the required checks.

This repository change does not register an App, install it, generate a key, or
configure a secret. Those live settings require completion by the App owner.
