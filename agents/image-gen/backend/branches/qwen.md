# База Qwen 2.1

Правила стека — инжектятся движком ТОЛЬКО при активном Qwen-бандле
(`base=qwen` в `[АКТИВНЫЙ БОНДЛ]`).

## Генерация (t2i)

Qwen Image 2.1 любит развёрнутые промпты: субъект, композиция, ракурс,
свет, атмосфера, стиль названы явно. Turbo-пресет — 8 шагов, CFG 1.0,
euler.

## Правка: теги `<imageN>` (обязательны при двух и более референсах)

У Qwen-стека нативный ref-conditioning: движок передаёт референсы в
порядке массива `source_image_ids` и подставляет перед каждой метку
`<image1>`, `<image2>`, … Теги в тексте промпта — то, как модель
понимает, КАКАЯ картинка какая.

- **Два и более референса:** адресуй картинки ТОЛЬКО тегами `<image1>`,
  `<image2>`, … Никаких «первая картинка», «вторая картинка», «image A» —
  модель такие формулировки не связывает с конкретным референсом.
- **Один референс:** теги НЕ пиши, обращайся естественно («the image»,
  «the person»).
- Роль каждой картинки проговори явно в промпте («the canvas is
  <image1>, the face comes from <image2>») — по официальному рецепту
  Qwen это обязательно.

## Правка: перенос идентичности (лицо, человек, товар)

Идентичность — самое хрупкое, что есть в правке. Правило: **указывай на
референс тегом, а НЕ описывай признаки словами**. Словесное описание
лица заставляет модель перегенерировать лицо и испортить сходство.
Замена лица (face-swap) на Qwen работает нативно — это её правильный
сценарий.

- Замена лица: `Replace the face and facial identity of the person in <image1> with the face and identity of the person in <image2>. Keep the head angle, expression, body pose, clothing, hairstyle, background and lighting of <image1> completely unchanged.`
- Замена головы целиком (лицо + причёска): `Replace the entire head of the person in <image1> with the head of the person in <image2>, matching <image1>'s head angle and expression. Keep <image1>'s body pose, clothing, composition, lighting and background unchanged.`
- Несколько людей в один кадр (все референсы — источники identities, холста нет): `<image1> and <image2> are two different people. Place them side by side, hugging, looking at the camera and smiling. Photorealistic portrait, consistent lighting.`
- Одежда/предмет: `Dress the person in <image1> with the outfit shown in <image2>. Keep the face, pose and background of <image1> unchanged.`

## Лоры базы Qwen

- `qwen2_1-femaledVaginus-nsfw-v1_3-fine_L22-31` — генеральная
  NSFW-лора, стартовая сила 1.0;
- `NSFW Qwen by TheseAlpacas V2` — NSFW-концепт (t2i и правки),
  стартовая сила 0.8;
- `Qwen-Image-2.1 NSFW Image Edit` — edit-лора правок, стартовая сила 1.0.

Взрослый контент (обнажённая анатомия, секс, эротика, интимные позы) →
стек из этих лор с явными силами объектами. Остальные запросы — без
`lora`. Стартовые силы калибруются по результатам боевых прогонов,
произвольно не меняй.
