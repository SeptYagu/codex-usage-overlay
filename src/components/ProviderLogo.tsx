import React, { useId } from 'react';
import { Provider } from '../types';

interface ProviderLogoProps {
  provider: Provider;
  size?: number;
}

// Brand marks identify the provider whose limits are shown. The logos are
// trademarks of their owners.
//
// Claude: Simple Icons 16.36.0, `icons/claude.svg` (CC0-1.0),
// https://github.com/simple-icons/simple-icons
const CLAUDE_PATH = 'm4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z';
const CLAUDE_COLOR = '#D97757';

// OpenAI (ChatGPT / Codex): LobeHub Icons, @lobehub/icons-static-svg 1.101.0,
// `icons/openai.svg`, https://github.com/lobehub/lobe-icons
// MIT License, Copyright (c) 2023 LobeHub. Permission is hereby granted, free
// of charge, to any person obtaining a copy of this software and associated
// documentation files, to deal in the Software without restriction, subject
// to including this notice; the software is provided "as is", without warranty.
const OPENAI_PATH = 'M9.205 8.658v-2.26c0-.19.072-.333.238-.428l4.543-2.616c.619-.357 1.356-.523 2.117-.523 2.854 0 4.662 2.212 4.662 4.566 0 .167 0 .357-.024.547l-4.71-2.759a.797.797 0 00-.856 0l-5.97 3.473zm10.609 8.8V12.06c0-.333-.143-.57-.429-.737l-5.97-3.473 1.95-1.118a.433.433 0 01.476 0l4.543 2.617c1.309.76 2.189 2.378 2.189 3.948 0 1.808-1.07 3.473-2.76 4.163zM7.802 12.703l-1.95-1.142c-.167-.095-.239-.238-.239-.428V5.899c0-2.545 1.95-4.472 4.591-4.472 1 0 1.927.333 2.712.928L8.23 5.067c-.285.166-.428.404-.428.737v6.898zM12 15.128l-2.795-1.57v-3.33L12 8.658l2.795 1.57v3.33L12 15.128zm1.796 7.23c-1 0-1.927-.332-2.712-.927l4.686-2.712c.285-.166.428-.404.428-.737v-6.898l1.974 1.142c.167.095.238.238.238.428v5.233c0 2.545-1.974 4.472-4.614 4.472zm-5.637-5.303l-4.544-2.617c-1.308-.761-2.188-2.378-2.188-3.948A4.482 4.482 0 014.21 6.327v5.423c0 .333.143.571.428.738l5.947 3.449-1.95 1.118a.432.432 0 01-.476 0zm-.262 3.9c-2.688 0-4.662-2.021-4.662-4.519 0-.19.024-.38.047-.57l4.686 2.71c.286.167.571.167.856 0l5.97-3.448v2.26c0 .19-.07.333-.237.428l-4.543 2.616c-.619.357-1.356.523-2.117.523zm5.899 2.83a5.947 5.947 0 005.827-4.756C22.287 18.339 24 15.84 24 13.296c0-1.665-.713-3.282-1.998-4.448.119-.5.19-.999.19-1.498 0-3.401-2.759-5.947-5.946-5.947-.642 0-1.26.095-1.88.31A5.962 5.962 0 0010.205 0a5.947 5.947 0 00-5.827 4.757C1.713 5.447 0 7.945 0 10.49c0 1.666.713 3.283 1.998 4.448-.119.5-.19 1-.19 1.499 0 3.401 2.759 5.946 5.946 5.946.642 0 1.26-.095 1.88-.309a5.96 5.96 0 004.162 1.713z';

// Google Gemini (Antigravity): 4-pointed colour-masked sparkle from the
// supplied Gemini logo asset (`viewBox="0 0 65 65"` star region).
const GEMINI_STAR_MASK_PATH = 'M32.4473 0C33.1278 0 33.7197 0.464783 33.8857 1.125C34.3947 3.14441 35.0586 5.11414 35.8848 7.03027C38.0369 12.0299 40.99 16.406 44.7393 20.1553C48.4903 23.9045 52.8647 26.8576 57.8643 29.0098C59.7821 29.8359 61.7502 30.4998 63.7695 31.0088C64.4297 31.1748 64.8944 31.7668 64.8945 32.4473C64.8945 33.1278 64.4298 33.7198 63.7695 33.8857C61.7502 34.3947 59.7803 35.0586 57.8643 35.8848C52.8646 38.037 48.4885 40.99 44.7393 44.7393C40.99 48.4904 38.037 52.8646 35.8848 57.8643C35.0586 59.7822 34.3947 61.7502 33.8857 63.7695C33.7198 64.4298 33.1278 64.8945 32.4473 64.8945C31.7668 64.8944 31.1748 64.4297 31.0088 63.7695C30.4998 61.7502 29.8359 59.7803 29.0098 57.8643C26.8576 52.8647 23.9063 48.4885 20.1553 44.7393C16.4041 40.99 12.0299 38.0369 7.03027 35.8848C5.1123 35.0586 3.14441 34.3947 1.125 33.8857C0.464783 33.7197 0 33.1278 0 32.4473C8.67651e-05 31.7668 0.464826 31.1748 1.125 31.0088C3.14442 30.4998 5.11413 29.836 7.03027 29.0098C12.03 26.8575 16.406 23.9046 20.1553 20.1553C23.9046 16.406 26.8575 12.03 29.0098 7.03027C29.836 5.11229 30.4998 3.14442 31.0088 1.125C31.1748 0.464826 31.7668 8.67651e-05 32.4473 0Z';
const GEMINI_HIGHLIGHT_PATH = 'M54.9838 -2.33625C57.8168 1.51558 54.1765 9.00477 46.8529 14.3913C39.5293 19.7779 31.2957 21.022 28.4627 17.1702C25.6296 13.3184 29.27 5.82919 36.5935 0.442635C43.9171 -4.94392 52.1507 -6.18807 54.9838 -2.33625Z';
const GEMINI_FILTERS: ReadonlyArray<readonly [x: number, y: number, width: number, height: number, stdDeviation: number]> = [
  [-19.8236, 13.1523, 39.2739, 43.2171, 2.45965],
  [-15.001, -40.257, 84.8677, 85.6878, 11.8911],
  [-20.7758, 11.9273, 79.454, 90.9166, 10.1086],
  [-20.7758, 11.9273, 79.454, 90.9166, 10.1086],
  [-19.8449, 15.459, 79.7306, 81.5048, 10.1086],
  [29.8324, -11.5524, 75.1172, 73.7582, 9.60613],
  [-38.5827, -16.2526, 78.1352, 78.7578, 8.70591],
  [8.1068, -5.96578, 78.877, 77.5394, 7.77473],
  [13.5873, -18.4881, 56.2718, 51.8102, 6.95694],
  [-15.5259, -31.297, 70.8565, 69.3059, 5.87598],
  [-14.1676, 20.9644, 55.5015, 51.5714, 7.27253],
];

export const ProviderLogo: React.FC<ProviderLogoProps> = ({ provider, size = 18 }) => {
  const uid = `antigravity-gemini-${useId().replace(/[^a-zA-Z0-9_-]/g, '')}`;
  const maskId = `${uid}-mask`;
  const paintId = `${uid}-paint`;
  const filterId = (index: number) => `${uid}-filter-${index}`;

  return (
    <svg
      width={size}
      height={size}
      viewBox={provider === 'antigravity' ? '0 0 65 65' : '0 0 24 24'}
      aria-hidden="true"
      focusable="false"
      data-provider={provider}
    >
      {provider === 'dsh'
        ? <><rect x="0.5" y="3" width="23" height="18" rx="4" fill="currentColor" fillOpacity="0.12" />
          <text x="12" y="15.5" textAnchor="middle" fill="currentColor" fontSize="8" fontFamily="system-ui, sans-serif" fontWeight="800">DSH</text></>
        : provider === 'claude'
        ? <path d={CLAUDE_PATH} fill={CLAUDE_COLOR} />
        : provider === 'antigravity'
        ? <>
            <mask id={maskId} style={{ maskType: 'alpha' }} maskUnits="userSpaceOnUse" x="0" y="0" width="65" height="65">
              <path d={GEMINI_STAR_MASK_PATH} fill="black" />
              <path d={GEMINI_STAR_MASK_PATH} fill={`url(#${paintId})`} />
            </mask>
            <g mask={`url(#${maskId})`}>
              {/* White base inside the mask keeps the blurred colour field vibrant without perimeter fringe. */}
              <rect x="0" y="0" width="65" height="65" fill="white" />
              <g filter={`url(#${filterId(0)})`}>
                <ellipse cx="14.4072" cy="16.9504" rx="14.4072" ry="16.9504" transform="matrix(0.942341 0.334654 -0.334652 0.942342 -8.09058 13.9664)" fill="#FFE432" />
              </g>
              <g filter={`url(#${filterId(1)})`}>
                <ellipse cx="27.4329" cy="2.5869" rx="18.6516" ry="19.0617" fill="#FC413D" />
              </g>
              <g filter={`url(#${filterId(2)})`}>
                <ellipse cx="18.9512" cy="57.3856" rx="19.4934" ry="25.2529" transform="rotate(-2.79865 18.9512 57.3856)" fill="#00B95C" />
              </g>
              <g filter={`url(#${filterId(3)})`}>
                <ellipse cx="18.9512" cy="57.3856" rx="19.4934" ry="25.2529" transform="rotate(-2.79865 18.9512 57.3856)" fill="#00B95C" />
              </g>
              <g filter={`url(#${filterId(4)})`}>
                <ellipse cx="20.0204" cy="56.2114" rx="19.1065" ry="21.0345" transform="rotate(-31.3178 20.0204 56.2114)" fill="#00B95C" />
              </g>
              <g filter={`url(#${filterId(5)})`}>
                <ellipse cx="67.391" cy="25.3267" rx="18.3463" ry="17.6668" fill="#3186FF" />
              </g>
              <g filter={`url(#${filterId(6)})`}>
                <ellipse cx="21.222" cy="22.3842" rx="21.222" ry="22.3842" transform="matrix(0.795985 0.605316 -0.605314 0.795987 -2.85815 -7.53723)" fill="#FBBC04" />
              </g>
              <g filter={`url(#${filterId(7)})`}>
                <ellipse cx="24.4687" cy="22.6039" rx="24.4687" ry="22.6039" transform="matrix(0.824033 0.566542 -0.566539 0.824035 40.1882 0.315002)" fill="#3186FF" />
              </g>
              <g filter={`url(#${filterId(8)})`}>
                <path d={GEMINI_HIGHLIGHT_PATH} fill="#749BFF" />
              </g>
              <g filter={`url(#${filterId(9)})`}>
                <ellipse cx="19.9023" cy="3.35597" rx="27.9712" ry="17.3877" transform="rotate(-42.848 19.9023 3.35597)" fill="#FC413D" />
              </g>
              <g filter={`url(#${filterId(10)})`}>
                <ellipse cx="13.5831" cy="46.7501" rx="14.9887" ry="8.71667" transform="rotate(35.592 13.5831 46.7501)" fill="#FFEE48" />
              </g>
            </g>
            <defs>
              {GEMINI_FILTERS.map(([x, y, width, height, stdDeviation], index) => (
                <filter
                  key={index}
                  id={filterId(index)}
                  x={x}
                  y={y}
                  width={width}
                  height={height}
                  filterUnits="userSpaceOnUse"
                  colorInterpolationFilters="sRGB"
                >
                  <feFlood floodOpacity="0" result="BackgroundImageFix" />
                  <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
                  <feGaussianBlur stdDeviation={stdDeviation} result="effect1" />
                </filter>
              ))}
              <linearGradient id={paintId} x1="18.4474" y1="43.4202" x2="52.1528" y2="15.0035" gradientUnits="userSpaceOnUse">
                <stop stopColor="#4893FC" />
                <stop offset="0.27" stopColor="#4893FC" />
                <stop offset="0.776981" stopColor="#969DFF" />
                <stop offset="1" stopColor="#BD99FE" />
              </linearGradient>
            </defs>
          </>
        // In the text colour, so it follows the light and dark themes.
        : <path d={OPENAI_PATH} fill="currentColor" fillRule="evenodd" />}
    </svg>
  );
};
