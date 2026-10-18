# Notices

Z-UI is licensed under the GNU General Public License, version 3 or later
(see `LICENSE`).

## Code from other projects

Only the dashboard (TypeScript UI) borrows code; the backend and everything
under it is Z-UI's own Rust, with no code from PasarGuard (Python) or 3X-UI
(Go).

Some dashboard files are adapted from **PasarGuard**
(<https://github.com/PasarGuard/panel>), © PasarGuard organization, licensed
under the **GNU Affero General Public License, version 3**. GPLv3 §13 allows
combining them with this GPLv3 work; those files keep their AGPLv3 terms and
say so in their first lines (`Adapted from PasarGuard … AGPL-3.0`):

- `dashboard/src/lib/users.ts` (user status colours)
- `dashboard/src/components/user-cells.tsx` (status badge, online dot, usage bar)
- `dashboard/src/pages/users.tsx` (users table and bulk bar)
- `dashboard/src/pages/templates.tsx` (template cards)

What that means for you: the AGPL's network clause applies to the combined
work. If you run a modified Z-UI for others to use over a network, you must
offer them its source code. Running it unmodified is covered by this
repository being public.

## Dependencies

- zray-core (`resources/zray-core`), MIT.
- Its vendored rustls fork, MPL-2.0; netstack-smoltcp, MIT or Apache-2.0.
- The dashboard's npm packages (React, Radix UI, Tailwind CSS, TanStack,
  i18next, lucide) are MIT or ISC; the Vazirmatn font is OFL-1.1.
