import { modelBrand } from '@shared/models';
import { MarkSvg } from './marks';

// Vendor logo for a model family ("GLM-5.2" → Zhipu mark). brand is the pre-resolved mark key — from the option's
// wire id via groupModels / optionBrand, since display names ("K3") are only labels; without one the family name is tried
export function ModelMark({ family, brand, className }: { family: string; brand?: string; className?: string }) {
  return <MarkSvg id={brand ?? modelBrand(family)} name={family} className={className} />;
}
