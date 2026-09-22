/// Статическое объявление типов для импорта статичных ассетов (logo.svg).
declare module "*.svg" {
  const src: string;
  export default src;
}

/// Сырой текст SVG (инлайн-рендер с поддержкой CSS-переменных в логотипе).
declare module "*.svg?raw" {
  const src: string;
  export default src;
}