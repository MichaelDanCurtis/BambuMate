use leptos::prelude::*;

#[component]
pub fn BrandMark() -> impl IntoView {
    view! {
        <div class="brand-mark" aria-hidden="true">
            <svg viewBox="0 0 96 96" class="brand-mark-svg">
                <defs>
                    <linearGradient id="brandCore" x1="18" y1="12" x2="78" y2="84" gradientUnits="userSpaceOnUse">
                        <stop offset="0" stop-color="#34c400" />
                        <stop offset="1" stop-color="#009a39" />
                    </linearGradient>
                    <linearGradient id="brandGlow" x1="20" y1="20" x2="76" y2="76" gradientUnits="userSpaceOnUse">
                        <stop offset="0" stop-color="#ffffff" stop-opacity="0.95" />
                        <stop offset="1" stop-color="#eaffe7" stop-opacity="0.22" />
                    </linearGradient>
                </defs>
                <rect x="10" y="10" width="76" height="76" rx="24" fill="url(#brandCore)" />
                <path d="M28 30h18c11.6 0 20 6.8 20 17.1 0 10.9-9.3 18.9-22 18.9H28V30Zm14.9 27.4c8.2 0 13.7-3.8 13.7-10.5 0-6.2-4.9-9.7-12.9-9.7h-6.5v20.2h5.7Z" fill="#06273a" />
                <path d="M54.5 29.5h13.8l-8.2 16.6 8.7 19.9H57.2l-4.6-11.4-5.2 11.4H36.2l10-19.8-8.8-16.7h12.1l4 9.4 5-9.4Z" fill="url(#brandGlow)" />
                <path d="M30 73c8.8-7.6 17.8-11.4 27-11.4 7.1 0 12.5 1.6 16.3 4.7" fill="none" stroke="rgba(255,255,255,0.52)" stroke-width="4" stroke-linecap="round" />
            </svg>
        </div>
    }
}

#[component]
pub fn HeroArtwork() -> impl IntoView {
    view! {
        <div class="hero-artwork" aria-hidden="true">
            <svg viewBox="0 0 520 320" class="hero-artwork-svg">
                <defs>
                    <linearGradient id="heroShell" x1="68" y1="30" x2="422" y2="274" gradientUnits="userSpaceOnUse">
                        <stop offset="0" stop-color="#34c400" />
                        <stop offset="1" stop-color="#00ae42" />
                    </linearGradient>
                    <linearGradient id="heroPanel" x1="148" y1="86" x2="364" y2="250" gradientUnits="userSpaceOnUse">
                        <stop offset="0" stop-color="#fdfffb" />
                        <stop offset="1" stop-color="#eef8ea" />
                    </linearGradient>
                </defs>
                <rect x="64" y="26" width="364" height="236" rx="34" fill="url(#heroShell)" />
                <rect x="94" y="56" width="304" height="176" rx="26" fill="#102417" fill-opacity="0.24" />
                <rect x="146" y="84" width="220" height="140" rx="22" fill="url(#heroPanel)" />
                <circle cx="194" cy="124" r="24" fill="#00ae42" />
                <path d="M176 123h36M194 105v36" stroke="#12351f" stroke-width="7" stroke-linecap="round" />
                <rect x="232" y="108" width="98" height="14" rx="7" fill="#87c774" />
                <rect x="232" y="134" width="72" height="14" rx="7" fill="#c7e0b8" />
                <rect x="232" y="160" width="112" height="14" rx="7" fill="#9fd38e" />
                <path d="M111 284c33-26 76-39 129-39 44 0 87 10 130 29" fill="none" stroke="#6ad35b" stroke-opacity="0.34" stroke-width="22" stroke-linecap="round" />
                <path d="M346 246c34-30 51-67 51-110" fill="none" stroke="#ffffff" stroke-opacity="0.55" stroke-width="12" stroke-linecap="round" />
                <circle cx="400" cy="90" r="23" fill="#ffffff" fill-opacity="0.2" />
                <circle cx="433" cy="124" r="11" fill="#ffffff" fill-opacity="0.34" />
            </svg>
        </div>
    }
}
