# Images live inside the Work's database

A Work's images (maps, illustrations) are stored inside its `.db`, and adding or replacing one is a Change. That keeps the Work a single file you can hand over, and old Milestones show the images they had at the time. To keep the Text file compilable outside the app, the app also writes the images next to it as regenerated copies (`<Work>.images/`), but only for Works that have images. Images are stored once per unique content, so inserting the same image twice or reverting to an old one adds nothing.

## Considered Options

- **Images as normal files in a folder next to the Work.** Simpler and editable in place, but the Work stops being one file you can share, and old Milestones would show today's images.

## Consequences

- History is append-only, so replacing a large image many times grows the file. The guidance is to iterate on images outside the Vault and insert the finished version. A print cover usually lives outside the Work entirely.
