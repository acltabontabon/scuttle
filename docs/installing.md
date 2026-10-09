# Installing Scuttle

Scuttle is built without paid code-signing certificates: it is **not signed with
an Apple Developer ID and not notarized** on macOS, and the Windows installer is
**unsigned**. Neither operating system can tell you who made it, so both will say
so the first time. Here is what you will see, and what to do.

**macOS.** Open the `.dmg` and drag Scuttle to Applications, then open it from
Applications. You will be told the developer cannot be verified. Go to **System
Settings → Privacy & Security**, scroll down to **Security**, and click **Open
Anyway** next to Scuttle, then open the app again. That button only appears
after you have tried to open the app, and only for about an hour afterwards, so
if you have wandered off and come back, try opening Scuttle once more first. You
do this once.

The macOS builds are *ad-hoc* signed, which is what lets an Apple silicon build
launch at all. Ad-hoc signing is not a verified publisher identity and it is not
notarization — it only means the bundle is internally consistent. If macOS tells
you Scuttle is **damaged** rather than unverified, that is not the same prompt
and it is not something to click past: the download is broken or was tampered
with. Check it against its `.sha256`, download it again, and
[open an issue](https://github.com/acltabontabon/scuttle/issues) if it persists.

**Windows.** Run the `-setup.exe`. Microsoft Defender SmartScreen will say it
prevented an unrecognised app from starting: click **More info**, then **Run
anyway**. Scuttle installs for your user account only, so Windows will not ask
for an administrator password. You will see this warning again on future
versions — SmartScreen's reputation is per file, and an unsigned project never
accumulates any.

Please don't switch off Gatekeeper, SmartScreen or your antivirus to install
this, or anything else. Nothing above asks you to change a system setting.


[Back to Scuttle](../README.md) · [Latest downloads](https://github.com/acltabontabon/scuttle/releases/latest)
