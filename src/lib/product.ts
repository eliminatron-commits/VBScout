// The central product configuration (name, URLs, prices, limits). Never
// hard-code any of these values in the UI.
import productJson from '../../product.json';

export const product = productJson;
export type Product = typeof productJson;
