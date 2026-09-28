import { lazy } from 'react';
let pending: Promise<typeof import('./provider')> | undefined;
const Calendar = lazy(() => (pending ??= import('./provider')).then(module => ({ default: module.Calendar })));
export function Picker() { return <Calendar />; }
