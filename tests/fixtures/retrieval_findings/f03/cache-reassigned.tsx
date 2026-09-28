import { lazy } from 'react';
let pending: Promise<typeof import('./provider')> | undefined;
pending = Promise.resolve(import('./reference'));
const CachedCalendar = lazy(() => (pending ??= import('./provider')).then(module => ({ default: module.Calendar })));
export function CacheReassignedPicker() { return <CachedCalendar />; }
