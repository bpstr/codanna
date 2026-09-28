import { lazy } from 'react';
const ProviderCalendar = lazy(() => import('./provider').then(module => ({ default: module.Calendar })));
export function ShadowedPicker(ProviderCalendar: () => null) { return <ProviderCalendar />; }
