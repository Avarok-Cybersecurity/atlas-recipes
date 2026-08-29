# Security

## Reporting

Email security@atlasinference.io. Please do not open a public issue for an
unpatched vulnerability.

## Why `atlasctl` exists

This launcher replaces `sparkrun`. The reason is not preference.

sparkrun 0.3.6 ships a hardcoded rewrite (`core/registry.py:545`) that silently
redirects the registry URL `Avarok-Cybersecurity/atlas-recipes` — this
repository — to `Atlas-Inf/sparkrun-recipes`, an organisation Atlas does not
control. It ships that URL as a baked-in default with `trusted=True`, and it
reserves the registry name `atlas` to that organisation, so the name cannot be
reclaimed from inside the tool.

`trusted` is not cosmetic. A trusted registry's recipes may carry
`post_commands`, which sparkrun runs on the host through a shell with no
prompt; `pre_exec`, `post_exec` and `mods` run as root inside the container with
no trust gate at all.

**If you have sparkrun installed**, run `atlasctl doctor`. Note that editing
sparkrun's config file does not fix the redirect: it is compiled into the tool
and reapplied on the next run. Removing the tool is the fix.

`doctor` exits **0** when it finds nothing and **1** when it finds something, so
this check can be gated on rather than read by eye — in a cron job, a
provisioning script, or CI:

```sh
atlasctl doctor || echo "this machine needs attention before it runs anything"
```

## What `atlasctl` does differently

- **Recipes are compiled in.** There is no fetch step to redirect. A fresh
  install performs no network access to resolve a recipe.
- **There is no trust flag.** Not in the code, not in the config schema. A
  remote registry supplies recipe data; it can never cause a command to run.
- **Recipe-supplied code is refused, not sandboxed.** `pre_exec`, `post_exec`,
  `post_commands`, `stop_after_post`, `mods`, `builder` and `builder_config`
  block a launch wherever they appear — including in recipes we ship. So does
  `executor_config`, because container isolation comes from one reviewed
  profile rather than from recipe data.
- **No shell, anywhere.** Commands are built and executed as argv vectors. A
  hostile value in a recipe is one inert argument, not a command.
- **No telemetry.** The tool makes no outbound request you did not ask for.
- **Registry names are resolved locally.** No external party decides who may
  use a name, and a bare recipe name resolves to a built-in recipe first, so a
  remote cannot shadow a shipped one.
- **Releases carry provenance.** Artifacts are checksummed and signed with
  Sigstore build attestations; `install.sh` verifies the checksum always, and
  the attestation when `gh` is available.

## Trust boundaries, stated plainly

`atlasctl` runs `docker`. **On Linux, membership of the `docker` group is
root-equivalent.** atlasctl does not raise your privileges — it exercises ones
you have already granted Docker — but it cannot be safer than that boundary.

A remote registry you add can name any container image. It runs under the same
unprivileged profile as everything else, but an image you do not trust is code
you do not trust. Add registries you would accept code from.

**On Windows, the secrets are protected by a directory, not by a mode.**
`browser.token` and `agent.key` live under `%LOCALAPPDATA%\atlasctl`, whose
inherited ACL grants the owning user, SYSTEM and Administrators — the same
trust boundary as `~/.config` at `0700`, where root reads everything too. There
is no mode bit to set and no equivalent of the unix check that a token has not
been widened, so what atlasctl verifies instead is *containment*: a secret must
live inside your user profile, and one destined elsewhere is refused before the
bytes are written. That is a weaker statement than the unix check and is not
presented as an equivalent one. If you point `--config-dir` at a network share
or a directory you have widened yourself, atlasctl will refuse the share and
cannot detect the second case.

`%LOCALAPPDATA%` and not `%APPDATA%` is deliberate: `%APPDATA%` roams between
machines on a domain, and `agent.key` is *this machine's* identity. Roaming it
would give two machines one node identity, which is sharing a private key by
copying a directory.

## The join code

Adding a machine to a fleet uses an 8-digit code. It is worth being explicit
about what that code is, because it is the one credential a human carries
between machines.

**It travels TO the target.** The machine being added dials the machine that
minted the code, so the code is pasted into a shell on the *new* machine — which
means it lands in that shell's history. That is why it is time-boxed rather than
durable.

What bounds it:

* **10 minutes.** After that the window is shut whether or not it was used.
* **Single use.** A code that completed a pairing cannot complete a second.
* **Three attempts, then a 60-second lockout.** A wrong code is not a free
  guess, so 8 digits are not brute-forced in the window they are alive for.
* **Revocable.** Closing the window in the browser takes effect immediately.

What the code does *not* do is authenticate on its own. It authorises the
ceremony; the ceremony itself is SPAKE2 over TLS bound to the channel, so a
relay that sits in the middle fails key confirmation rather than learning
anything. The code is what decides that a stranger may attempt the ceremony at
all — outside an open window, an unpinned machine reaches no further than the
TLS handshake.

**A pairing is not a launch grant.** Pairing establishes identity. Whether a
machine that joins may also *drive* this one is a separate flag, chosen when the
code is minted rather than implied by the pairing.

**The two directions confirm at different moments, and it is worth knowing
which you are in.** When you pair a machine you can already see — the
browser-driven flow — nothing is written until a human compares the
verification words and confirms; the exchange is held until then, and if it is
never confirmed no pin is written and there is nothing to undo. When a machine
joins with a code, that machine writes its pin as soon as the ceremony
succeeds: the code it carried *is* the authorisation, and the words are printed
so the operator can compare them afterwards. If they do not match, unpair —
that is a smaller window than the browser flow offers, and it is the price of a
one-shot code that nobody has to be present to accept.

## Reproducing the parity claim

Serve commands are byte-identical to the reference implementation's across the
whole recipe corpus. See [docs/PARITY.md](docs/PARITY.md) for how that was
measured, and for the differences that are deliberate.
