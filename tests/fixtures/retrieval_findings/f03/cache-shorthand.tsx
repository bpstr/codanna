import { lazy } from 'react';
let pending: Promise<typeof import('./provider')> | undefined;
({ pending } = { pending: Promise.resolve(import('./reference')) });
const ShorthandCalendar = lazy(() => (pending ??= import('./provider')).then(module => ({ default: module.Calendar })));
export function ShorthandCachePicker() { return <ShorthandCalendar />; }
