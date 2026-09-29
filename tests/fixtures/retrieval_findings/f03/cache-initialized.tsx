import { lazy } from 'react';
const undefined = Promise.resolve(import('./reference'));
let pending = undefined;
const InitializedCalendar = lazy(() => (pending ??= import('./provider')).then(module => ({ default: module.Calendar })));
export function InitializedCachePicker() { return <InitializedCalendar />; }
