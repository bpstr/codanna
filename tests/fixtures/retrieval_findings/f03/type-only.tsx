import { lazy } from 'react';
const TypeOnlyCalendar = lazy(() => import('./types').then(module => ({ default: module.Calendar })));
export function TypeOnlyPicker() { return <TypeOnlyCalendar />; }
