// Patchs BlueOrb :
// 1. Recompilation incrémentale C (`3dsrecomp`) + compilation pleine vitesse multi-cœurs
// 2. Améliorations Remaster HD Pokémon Saphir Alpha (Upscaling 4x Smooth-xBR, Anisotropie 16x, No Outlines HD)
// 3. Correction complète du clignotement 60 Hz / 60 FPS (désactivation par défaut de l'interpolation + protection des UV yeux/bouches)
// 4. Support Ultra-Wide Hor+ (16:9, 21:9, 32:9) & Double Fenêtre détachable (F7)
// 5. Assistant Premier Lancement ultra-simple (glisser-déposer / choisir la ROM .3ds -> recompilation auto -> lancement direct aux fois suivantes)

use std::fs;
use std::path::Path;

fn read_lf(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map(|s| s.replace("\r\n", "\n"))
        .map_err(|e| e.to_string())
}

pub fn apply_sdk_enhancements(recomp_repo: &Path, zakuro_repo: &Path) -> Result<(), String> {
    patch_recomp_incremental(recomp_repo)?;
    patch_zakuro_oras_enhancements(zakuro_repo)?;
    patch_zakuro_gpu_ultrawide_upscaler_antiflicker_and_dual_vulkan(zakuro_repo)?;
    patch_zakuro_frontend_onboarding_console_ui_and_dual_window(zakuro_repo)?;
    Ok(())
}

/// 1. Rend la compilation C de `3dsrecomp` incrémentale : si `.o` est plus récent que `.c` et non vide, on le réutilise.
fn patch_recomp_incremental(recomp_repo: &Path) -> Result<(), String> {
    let compile_rs = recomp_repo.join("src").join("compile.rs");
    if !compile_rs.exists() {
        return Ok(());
    }
    let src = read_lf(&compile_rs).map_err(|e| e.to_string())?;
    if src.contains("BLUEORB_INCREMENTAL_OBJ") {
        return Ok(());
    }

    let needle = "                let path = dir.join(source);\n                let mut command = compiler.command();";
    let replacement = r#"                let path = dir.join(source);
                // BLUEORB_INCREMENTAL_OBJ: réutiliser l'objet .o déjà compilé s'il est à jour
                let obj_path = path.with_extension("o");
                let up_to_date = std::fs::metadata(&obj_path)
                    .and_then(|om| Ok((om.len(), om.modified()?)))
                    .and_then(|(olen, otime)| Ok(olen > 64 && otime >= std::fs::metadata(&path)?.modified()?))
                    .unwrap_or(false);
                if up_to_date {
                    if !progress(done.fetch_add(1, Ordering::Relaxed) + 1, sources.len()) {
                        stopped.store(true, Ordering::Relaxed);
                    }
                    let mut state = state.lock().unwrap();
                    state.1 -= needs[index];
                    state.2 -= 1;
                    freed.notify_all();
                    continue;
                }
                let mut command = compiler.command();"#;

    if src.contains(needle) {
        fs::write(&compile_rs, src.replace(needle, replacement)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 2. Ajoute l'amélioration vérifiée "No Outlines (HD)" pour Pokémon Saphir Alpha USA Rev 2 (`rev3`) et Omega Ruby.
fn patch_zakuro_oras_enhancements(zakuro_repo: &Path) -> Result<(), String> {
    let enh_path = zakuro_repo
        .join("crates")
        .join("zakuro-core")
        .join("src")
        .join("enhancements.txt");
    if !enh_path.exists() {
        return Ok(());
    }
    let src = read_lf(&enh_path).map_err(|e| e.to_string())?;
    if src.contains("No Outlines (HD)") {
        return Ok(());
    }

    let target = "= 000400000011C500 Pokémon Alpha Sapphire (USA)\n60 FPS | rev3 | Reshiban | The whole game runs twice as fast with it.\n28C650C1 00000000";
    let updated = "= 000400000011C500 Pokémon Alpha Sapphire (USA)\n\
No Outlines (HD) | rev3 | BlueOrb | Removes black cel-shading outlines around 3D models for clean HD rendering.\n\
D3000000 00000000\n\
00379EB4 00000000\n\
00379EB8 00000000\n\
D2000000 00000000\n\
60 FPS | rev3 | Reshiban | Runs game logic at 2x speed.\n\
28C650C1 00000000";

    if src.contains(target) {
        fs::write(&enh_path, src.replace(target, updated)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 3. Patchs `zakuro-gpu` :
///    - Correction anti-clignotement 60 FPS dans `interpolation.rs` (ne jamais interpoler les uniformes d'atlas UV yeux/bouches/eau)
///    - Projection 3D Hor+ Ultra-Wide (`16:9`, `21:9`, `32:9`) sans étirement
///    - Upscaler de textures 4x Smooth-xBR/EPX + Mipmaps trilinéaires + Anisotropie 16x
///    - `VulkanPresenter::new_secondary` zéro-copie pour l'écran tactile détaché
fn patch_zakuro_gpu_ultrawide_upscaler_antiflicker_and_dual_vulkan(zakuro_repo: &Path) -> Result<(), String> {
    // 3a. `zakuro-gpu/src/interpolation.rs` : suppression du clignotement 60 Hz sur les textures/yeux/transitions
    let interp_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("interpolation.rs");
    if interp_rs.exists() {
        let mut src = read_lf(&interp_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_ANTIFLICKER_60HZ") {
            // Détecter aussi les pas d'atlas 1/3, 1/6, 1/12, 1/24 (yeux, bouches et sprites Pokémon 3DS) en plus des 1/64 !
            src = src.replace(
                "fn steps(before: f32, now: f32) -> bool {\n    let round = |value: f32| (value * 64.0).fract() == 0.0;\n    round(before) && round(now)\n}",
                "// BLUEORB_ANTIFLICKER_60HZ\nfn steps(before: f32, now: f32) -> bool {\n    let round = |value: f32| {\n        let s64 = value * 64.0;\n        let s24 = value * 24.0;\n        (s64 - s64.round()).abs() < 1e-5 || (s24 - s24.round()).abs() < 1e-5\n    };\n    round(before) && round(now)\n}",
            );
            src = src.replace(
                "        self.moved > 0 && self.matched * 2 >= self.draws && self.jumped * 4 <= self.matched",
                "        self.moved > 0 && self.matched * 4 >= self.draws * 3 && self.jumped * 8 <= self.matched",
            );
            fs::write(&interp_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // 3b. `zakuro-gpu/src/lib.rs`
    let gpu_lib_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("lib.rs");
    if gpu_lib_rs.exists() {
        let src = read_lf(&gpu_lib_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_TOP_ASPECT_SCALE") {
            let addition = r#"
static BLUEORB_TOP_ASPECT_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3F80_0000);

pub fn set_top_aspect_scale(top_width: f32) {
    let scale = if top_width > 400.5 { (400.0 / top_width).clamp(0.25, 1.0) } else { 1.0 };
    BLUEORB_TOP_ASPECT_SCALE.store(scale.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub fn top_aspect_scale() -> f32 {
    f32::from_bits(BLUEORB_TOP_ASPECT_SCALE.load(std::sync::atomic::Ordering::Relaxed))
}
"#;
            fs::write(&gpu_lib_rs, format!("{}{}", src, addition)).map_err(|e| e.to_string())?;
        }
    }

    // 3c. `zakuro-gpu/src/raster.rs`
    let raster_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("raster.rs");
    if raster_rs.exists() {
        let src = read_lf(&raster_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_ULTRAWIDE_PLACE") {
            let needle = "fn place(registers: &[u32], shaded: &[Vertex], order: Option<&[usize]>, viewport: (f32, f32, f32, f32)) -> Placement {";
            let replacement = r#"fn place(registers: &[u32], shaded_in: &[Vertex], order: Option<&[usize]>, viewport: (f32, f32, f32, f32)) -> Placement {
    // BLUEORB_ULTRAWIDE_PLACE
    let aspect_scale = crate::top_aspect_scale();
    let uw_storage: Vec<Vertex>;
    let shaded: &[Vertex] = if aspect_scale < 0.999 && (viewport.2 - 240.0).abs() < 1.0 && (viewport.3 - 400.0).abs() < 1.0 {
        let is_fullscreen_2d_quad = shaded_in.len() <= 6
            && shaded_in.iter().all(|v| (v.clip[3] - 1.0).abs() < 1e-3 && v.clip[1].abs() >= 0.98);
        if is_fullscreen_2d_quad {
            shaded_in
        } else {
            uw_storage = shaded_in
                .iter()
                .copied()
                .map(|mut v| {
                    v.clip[1] *= aspect_scale;
                    v
                })
                .collect();
            &uw_storage
        }
    } else {
        shaded_in
    };"#;
            if src.contains(needle) {
                fs::write(&raster_rs, src.replace(needle, replacement)).map_err(|e| e.to_string())?;
            }
        }
    }

    // 3d. `zakuro-gpu/src/shader/glsl.rs`
    let glsl_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("shader").join("glsl.rs");
    if glsl_rs.exists() {
        let src = read_lf(&glsl_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_GLSL_ULTRAWIDE") {
            let needle = "const EPILOGUE: &str = r#\"    if (position.w > 1e-5) {";
            let replacement = r#"const EPILOGUE: &str = r#"    // BLUEORB_GLSL_ULTRAWIDE
    float aspect_scale = uintBitsToFloat(pad[0]);
    if (aspect_scale > 0.1 && aspect_scale < 0.999 && abs(viewport.z - 240.0) < 1.0 && abs(viewport.w - 400.0) < 1.0) {
        if (!(abs(position.w - 1.0) < 1e-3 && abs(position.y) >= 0.98)) {
            position.y *= aspect_scale;
        }
    }
    if (position.w > 1e-5) {"#;
            if src.contains(needle) {
                fs::write(&glsl_rs, src.replace(needle, replacement)).map_err(|e| e.to_string())?;
            }
        }
    }

    // 3e. `zakuro-gpu/src/raster/hardware.rs`
    let hw_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("raster").join("hardware.rs");
    if hw_rs.exists() {
        let mut src = read_lf(&hw_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_HD_UPSCALE") {
            src = src.replace(
                "    max_bias: f32,\n",
                "    max_bias: f32,\n    max_anisotropy: f32,\n",
            );
            src = src.replace(
                "                max_bias: properties.limits.max_sampler_lod_bias,\n",
                "                max_bias: properties.limits.max_sampler_lod_bias,\n                max_anisotropy: if instance.get_physical_device_features(physical).sampler_anisotropy == vk::TRUE { properties.limits.max_sampler_anisotropy.min(16.0) } else { 1.0 },\n",
            );
            src = src.replace(
                "    let features = vk::PhysicalDeviceFeatures::default()\n        .depth_clamp(true)\n        .logic_op(logic_ops)\n        .pipeline_statistics_query(statistics);",
                "    let features = vk::PhysicalDeviceFeatures::default()\n        .depth_clamp(true)\n        .logic_op(logic_ops)\n        .sampler_anisotropy(supported.sampler_anisotropy == vk::TRUE)\n        .pipeline_statistics_query(statistics);",
            );
            src = src.replace(
                "            .min_lod(filtering.levels.0 as f32)\n            .max_lod(filtering.levels.1 as f32);",
                "            .anisotropy_enable(self.max_anisotropy > 1.0 && filtering.magnify && filtering.minify)\n            .max_anisotropy(if self.max_anisotropy > 1.0 && filtering.magnify && filtering.minify { self.max_anisotropy } else { 1.0 })\n            .min_lod(filtering.levels.0 as f32)\n            .max_lod(filtering.levels.1 as f32);",
            );
            src = src.replace(
                "                        None => {\n                            let (view, picture) = self.texture(bound)?;",
                "                        None => {\n                            let (view, picture) = self.texture_hd(bound)?;",
            );
            let target_fn = "    fn texture(&mut self, bound: &BoundTexture) -> Result<(vk::ImageView, Option<u32>), String> {";
            let hd_fn_and_orig = r#"    // BLUEORB_HD_UPSCALE: Upscaling temps-réel 4x Smooth-xBR/EPX + Mipmaps trilinéaires
    fn texture_hd(&mut self, bound: &BoundTexture) -> Result<(vk::ImageView, Option<u32>), String> {
        let size = (bound.width, bound.height);
        if let Some((view, smallest)) = bound.replacement.as_ref().and_then(|material| self.replacement(material, size)) {
            return Ok((view, Some(smallest)));
        }
        let can_upscale = self.scale >= 2
            && bound.levels == 1
            && bound.width >= 8
            && bound.height >= 8
            && bound.width <= 512
            && bound.height <= 512
            && (bound.width * 4) <= self.max_image_size
            && (bound.height * 4) <= self.max_image_size
            && std::env::var_os("BLUEORB_NO_TEX_UPSCALE").is_none();

        if can_upscale {
            let hd_key = (Arc::as_ptr(&bound.texels) as *const u8 as usize) | 1usize;
            if let Some(texture) = self.textures.get_mut(&hd_key) {
                texture.used = self.batch;
                return Ok((texture.image.view, Some(2)));
            }
            let pic = blueorb_upscale_4x_mipmapped(bound.width, bound.height, &bound.texels);
            if let Ok(image) = self.upload_picture(&pic) {
                let view = image.view;
                self.textures.insert(hd_key, Texture { image, _texels: bound.texels.clone(), used: self.batch });
                return Ok((view, Some(2)));
            }
        }
        self.texture(bound)
    }

    fn texture(&mut self, bound: &BoundTexture) -> Result<(vk::ImageView, Option<u32>), String> {"#;
            src = src.replace(target_fn, hd_fn_and_orig);
            src.push_str(UPSCALE_HELPERS_RS);
        }
        if !src.contains("crate::top_aspect_scale().to_bits()") {
            src = src.replace(
                "words.extend([unit.bool_uniforms as u32, unit.entry_point, 0, 0]);",
                "words.extend([unit.bool_uniforms as u32, unit.entry_point, crate::top_aspect_scale().to_bits(), 0]);",
            );
        }
        fs::write(&hw_rs, src).map_err(|e| e.to_string())?;
    }

    // 3f. `zakuro-gpu/src/backend/vulkan.rs`
    let vk_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("backend").join("vulkan.rs");
    if vk_rs.exists() {
        let mut src = read_lf(&vk_rs).map_err(|e| e.to_string())?;
        if !src.contains("pub fn new_secondary(") {
            src = src.replace(
                "    stale: bool,\n}",
                "    stale: bool,\n    secondary: bool,\n}",
            );
            src = src.replace(
                "            stale: false,\n        };",
                "            stale: false,\n            secondary: false,\n        };",
            );
            src = src.replace(
                "_ => vec![vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::FIFO],",
                "_ if self.secondary => vec![vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::IMMEDIATE, vk::PresentModeKHR::FIFO],\n            _ => vec![vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::FIFO],",
            );

            let sec_ctor = r#"    pub fn new_secondary(
        window: &(impl HasDisplayHandle + HasWindowHandle),
        size: (u32, u32),
        shared_dev: Arc<SharedDevice>,
    ) -> Result<VulkanPresenter, PresentError> {
        let entry = shared_dev._entry.clone();
        let instance = shared_dev.instance.clone();
        let physical_device = shared_dev.physical;
        let queue_family = shared_dev.family;
        let device = shared_dev.device.clone();
        let queue = shared_dev.queue;

        let display = window.display_handle().map_err(|e| fail(format!("no display handle: {e}")))?;
        let window_handle = window.window_handle().map_err(|e| fail(format!("no window handle: {e}")))?;
        let surface_instance = ash::khr::surface::Instance::new(&entry, &instance);
        let surface = unsafe {
            ash_window::create_surface(&entry, &instance, display.as_raw(), window_handle.as_raw(), None)
        }.map_err(vk_fail("creating secondary surface"))?;

        let memory_properties = unsafe { instance.get_physical_device_memory_properties(physical_device) };
        let swapchain_device = ash::khr::swapchain::Device::new(&instance, &device);
        let surface_format = choose_format(&surface_instance, physical_device, surface)?;
        let render_pass = create_render_pass(&device, surface_format.format)?;
        let descriptor_layout = create_descriptor_layout(&device, 1)?;
        let (pipeline_layout, pipeline) = create_pipeline(&device, render_pass, descriptor_layout, FRAGMENT_SPIRV, CONSTANTS)?;
        let stereo_layout = create_descriptor_layout(&device, 2)?;
        let (stereo_pipeline_layout, stereo_pipeline) = create_pipeline(&device, render_pass, stereo_layout, STEREO_FRAGMENT_SPIRV, STEREO_CONSTANTS)?;
        let srgb = matches!(surface_format.format, vk::Format::B8G8R8A8_SRGB | vk::Format::R8G8B8A8_SRGB);
        let overlay = OverlayPainter::new(&device, render_pass, FRAMES_IN_FLIGHT, srgb)?;

        let sampler = unsafe {
            device.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                None,
            )
        }.map_err(vk_fail("creating secondary sampler"))?;

        let descriptor_pool = unsafe {
            let sets = 6 + 4 * FRAMES_IN_FLIGHT as u32;
            let sizes = [vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(6 + 5 * FRAMES_IN_FLIGHT as u32)];
            device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
                    .pool_sizes(&sizes)
                    .max_sets(sets),
                None,
            )
        }.map_err(vk_fail("creating secondary descriptor pool"))?;

        let gpu_sets = unsafe {
            let layouts = [descriptor_layout; 3 * FRAMES_IN_FLIGHT];
            device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default().descriptor_pool(descriptor_pool).set_layouts(&layouts),
            )
        }.map_err(vk_fail("allocating secondary descriptor sets"))?
        .chunks(3)
        .map(|s| [s[0], s[1], s[2]])
        .collect();

        let stereo_sets = unsafe {
            let layouts = [stereo_layout; FRAMES_IN_FLIGHT];
            device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default().descriptor_pool(descriptor_pool).set_layouts(&layouts),
            )
        }.map_err(vk_fail("allocating secondary stereo sets"))?;

        let command_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
        }.map_err(vk_fail("creating secondary command pool"))?;

        let command_buffers = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(FRAMES_IN_FLIGHT as u32),
            )
        }.map_err(vk_fail("allocating secondary command buffers"))?;

        let mut image_available = Vec::with_capacity(FRAMES_IN_FLIGHT);
        let mut render_finished = Vec::with_capacity(FRAMES_IN_FLIGHT);
        let mut in_flight = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for _ in 0..FRAMES_IN_FLIGHT {
            let sem_info = vk::SemaphoreCreateInfo::default();
            image_available.push(unsafe { device.create_semaphore(&sem_info, None) }.map_err(vk_fail("creating semaphore"))?);
            render_finished.push(unsafe { device.create_semaphore(&sem_info, None) }.map_err(vk_fail("creating semaphore"))?);
            in_flight.push(unsafe {
                device.create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None)
            }.map_err(vk_fail("creating fence"))?);
        }

        let screens = [
            Screen::new(&device, &memory_properties, descriptor_pool, descriptor_layout, sampler, 400, 240)?,
            Screen::new(&device, &memory_properties, descriptor_pool, descriptor_layout, sampler, 320, 240)?,
            Screen::new(&device, &memory_properties, descriptor_pool, descriptor_layout, sampler, 400, 240)?,
        ];

        let mut presenter = VulkanPresenter {
            _entry: entry,
            instance,
            surface_instance,
            surface,
            physical_device,
            memory_properties,
            device,
            queue_family,
            queue,
            swapchain_device,
            swapchain: vk::SwapchainKHR::null(),
            surface_format,
            extent: vk::Extent2D { width: size.0, height: size.1 },
            image_views: Vec::new(),
            framebuffers: Vec::new(),
            render_pass,
            descriptor_layout,
            descriptor_pool,
            pipeline_layout,
            pipeline,
            stereo_layout,
            stereo_pipeline_layout,
            stereo_pipeline,
            stereo_sets,
            sampler,
            gpu_sets,
            shared: Some(shared_dev),
            waits: false,
            command_pool,
            command_buffers,
            image_available,
            render_finished,
            in_flight,
            frame: 0,
            screens,
            overlay: Some(overlay),
            window: size,
            arrangement: ScreenLayout::BottomOnly,
            orientation: ScreenOrientation::default(),
            top_width: super::TOP_WIDTH,
            filter: ScreenFilter::default(),
            integer: false,
            stereo: Stereo::default(),
            stale: false,
            secondary: true,
        };
        presenter.build_swapchain()?;
        Ok(presenter)
    }

    pub fn new("#;

            src = src.replace("    pub fn new(", sec_ctor);
            fs::write(&vk_rs, src).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

/// 4. Assistant Premier Lancement ultra-intuitif + Interface Console Large + Double Fenêtre
fn patch_zakuro_frontend_onboarding_console_ui_and_dual_window(zakuro_repo: &Path) -> Result<(), String> {
    let zakuro_crate = zakuro_repo.join("crates").join("zakuro");
    let zakuro_src = zakuro_crate.join("src");

    // 4a-bis. `build.rs` : embed AppIcon.ico into BlueOrb.exe on Windows MSVC when llvm-rc / rc is available
    let build_rs = zakuro_crate.join("build.rs");
    if build_rs.exists() {
        let src = read_lf(&build_rs).map_err(|e| e.to_string())?;
        if !src.contains("C:/Program Files/LLVM/bin/llvm-rc.exe") {
            let win_icon_hook = r#"
    // BLUEORB_WIN_ICON
    if windows && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
        let ico = manifest_dir.join("AppIcon.ico");
        println!("cargo:rerun-if-changed={}", ico.display());
        if ico.is_file() {
            if let Ok(out_dir) = std::env::var("OUT_DIR") {
                let out = std::path::PathBuf::from(out_dir);
                let rc_file = out.join("blueorb_icon.rc");
                let res_file = out.join("blueorb_icon.res");
                let ico_escaped = ico.display().to_string().replace('\\', "/");
                let _ = std::fs::write(&rc_file, format!("1 ICON \"{ico_escaped}\"\n"));
                let try_llvm_rc = |bin: &str| {
                    std::process::Command::new(bin)
                        .args(["/FO"])
                        .arg(&res_file)
                        .arg(&rc_file)
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                };
                let compiled = try_llvm_rc("llvm-rc")
                    || try_llvm_rc("C:/Program Files/LLVM/bin/llvm-rc.exe")
                    || std::process::Command::new("rc")
                        .args(["/nologo", "/fo"])
                        .arg(&res_file)
                        .arg(&rc_file)
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                if compiled && res_file.is_file() {
                    println!("cargo:rustc-link-arg-bins={}", res_file.display());
                }
            }
        }
    }
}"#;
            let insert_at = src
                .find("\n    // BLUEORB_WIN_ICON")
                .or_else(|| src.rfind('}'));
            if let Some(pos) = insert_at {
                let mut patched = src[..pos].to_string();
                patched.push_str(win_icon_hook);
                patched.push('\n');
                fs::write(&build_rs, patched).map_err(|e| e.to_string())?;
            }
        }
    }

    // 4a. `main.rs` : no black console window on Windows + offer to move into /Applications on macOS
    let main_rs = zakuro_src.join("main.rs");
    if main_rs.exists() {
        let mut src = read_lf(&main_rs).map_err(|e| e.to_string())?;
        if !src.contains("windows_subsystem") {
            src = format!("#![cfg_attr(target_os = \"windows\", windows_subsystem = \"windows\")]\n{src}");
        }
        if !src.contains("BLUEORB_MOVE_TO_APPLICATIONS") {
            src = src.replace(
                "    zakuro::run(None);",
                "    #[cfg(target_os = \"macos\")]\n    blueorb_offer_move_to_applications();\n    zakuro::run(None);",
            );
            src.push_str(MOVE_TO_APPLICATIONS_RS);
        }
        fs::write(&main_rs, src).map_err(|e| e.to_string())?;
    }

    // 4b. `recompile.rs` : compiler à pleine vitesse sur les cœurs Performance (4x-5x plus rapide au 1er lancement !)
    let recompile_rs = zakuro_src.join("recompile.rs");
    if recompile_rs.exists() {
        let src = read_lf(&recompile_rs).map_err(|e| e.to_string())?;
        if src.contains("background: true") {
            fs::write(&recompile_rs, src.replace("background: true", "background: false"))
                .map_err(|e| e.to_string())?;
        }
    }

    // 4c. `gui.rs` : Thème sombre sobre & lisible
    let gui_rs = zakuro_src.join("gui.rs");
    if gui_rs.exists() {
        let mut src = read_lf(&gui_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_CONSOLE_THEME") {
            src = src.replace(
                "        ctx.set_fonts(fonts(system_fonts()));",
                "        ctx.set_fonts(fonts(system_fonts()));\n        apply_blueorb_console_theme(&ctx);",
            );
            src.push_str(CONSOLE_THEME_FN_RS);
            fs::write(&gui_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // 4d. `settings.rs` : `blueorb_rom`, valeurs par défaut optimales (Français, 5x 1440p, No Outlines HD, 0 clignotement)
    let settings_rs = zakuro_src.join("settings.rs");
    if settings_rs.exists() {
        let mut src = read_lf(&settings_rs).map_err(|e| e.to_string())?;
        if !src.contains("ZAKURO_SETTINGS_PATH") {
            src = src.replace(
                "fn path() -> Option<PathBuf> {",
                "fn path() -> Option<PathBuf> {\n    if let Some(custom) = std::env::var_os(\"ZAKURO_SETTINGS_PATH\").filter(|v| !v.is_empty()) {\n        return Some(PathBuf::from(custom));\n    }",
            );
        }
        if !src.contains("blueorb_rom") {
            src = src.replace(
                "    pub games: Option<PathBuf>,",
                "    pub games: Option<PathBuf>,\n    pub blueorb_rom: Option<PathBuf>,",
            );
            src = src.replace(
                "            games: None,\n            language: Language::English,\n            renderer: Renderer::Vulkan,\n            hardware_rasterizer: true,\n            resolution: 3,",
                "            games: None,\n            blueorb_rom: None,\n            language: Language::English,\n            renderer: Renderer::Vulkan,\n            hardware_rasterizer: true,\n            resolution: 5,",
            );
            src = src.replace(
                "            filter: Filter::Smooth,",
                "            filter: Filter::Sharp,",
            );
            src = src.replace(
                "            enhancements: std::collections::BTreeMap::new(),",
                "            enhancements: std::collections::BTreeMap::from([(\"000400000011C500\".to_owned(), vec![\"No Outlines (HD)\".to_owned()])]),",
            );
        }
        if !src.contains("SeparateWindows") {
            src = src.replace(
                "    LargeTop,\n}",
                "    LargeTop,\n    SeparateWindows,\n}",
            );
            src = src.replace(
                "pub const ALL: [Screens; 5] = [Screens::Stacked, Screens::SideBySide, Screens::LargeTop, Screens::TopOnly, Screens::BottomOnly];",
                "pub const ALL: [Screens; 6] = [Screens::Stacked, Screens::SideBySide, Screens::LargeTop, Screens::SeparateWindows, Screens::TopOnly, Screens::BottomOnly];",
            );
            src = src.replace(
                "Screens::LargeTop => \"Big top screen, small bottom one\",",
                "Screens::LargeTop => \"Big top screen, small bottom one\",\n            Screens::SeparateWindows => \"Separate windows\",",
            );
            src = src.replace(
                "Screens::LargeTop => ScreenLayout::LargeTop,",
                "Screens::LargeTop => ScreenLayout::LargeTop,\n            Screens::SeparateWindows => ScreenLayout::TopOnly,",
            );
        }
        if !src.contains("Wide32x9") {
            src = src.replace(
                "    Wide21x9,\n}",
                "    Wide21x9,\n    Wide32x9,\n}",
            );
            src = src.replace(
                "pub const ALL: [TopShape; 3] = [TopShape::Console, TopShape::Wide16x9, TopShape::Wide21x9];",
                "pub const ALL: [TopShape; 4] = [TopShape::Console, TopShape::Wide16x9, TopShape::Wide21x9, TopShape::Wide32x9];",
            );
            src = src.replace(
                "TopShape::Wide21x9 => \"21:9\",",
                "TopShape::Wide21x9 => \"21:9 Ultrawide\",\n            TopShape::Wide32x9 => \"32:9 Super Ultrawide\",",
            );
            src = src.replace(
                "TopShape::Wide21x9 => 560.0,",
                "TopShape::Wide21x9 => 560.0,\n            TopShape::Wide32x9 => 240.0 * 32.0 / 9.0,",
            );
        }
        fs::write(&settings_rs, src).map_err(|e| e.to_string())?;
    }

    // 4e. `menus.rs` : Assistant d'accueil 1er lancement (`library`) + Menu Pause accessible (`game`)
    let menus_rs = zakuro_src.join("menus.rs");
    if menus_rs.exists() {
        let mut src = read_lf(&menus_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_WIDE_CONSOLE_MENU") {
            src = src.replace(
                "    CheatsFolder,\n}",
                "    CheatsFolder,\n    PickRomFile,\n    CleanResiduals,\n    InstallToApplications,\n    ToggleDualWindows,\n    SetLayout(Screens),\n    SetTopShape(TopShape),\n}",
            );
            src = src.replace(
                "    pub menu_open: bool,",
                "    pub menu_open: bool,\n    pub tab: u8,",
            );

            // Remplacement de `Menus::library` par l'Assistant Premier Lancement ultra-accessible
            let lib_start = "    /// the library, filling the window, waiting are the games waiting for their turn to recompile\n    pub fn library(";
            let game_start = "    /// what goes over a running game, the menu when it's open and the frame rate when asked for\n    #[allow(clippy::too_many_arguments)]\n    pub fn game(";
            let Some(i0) = src.find(lib_start) else {
                return Err("Impossible de localiser `Menus::library` dans `menus.rs`".into());
            };
            let Some(i1) = src.find(game_start) else {
                return Err("Impossible de localiser `Menus::game` dans `menus.rs`".into());
            };
            src.replace_range(i0..i1, ONBOARDING_LIBRARY_FN_RS);

            let old_game_fn = r#"    /// what goes over a running game, the menu when it's open and the frame rate when asked for
    #[allow(clippy::too_many_arguments)]
    pub fn game(
        &mut self,
        ui: &mut egui::Ui,
        name: &str,
        program_id: Option<u64>,
        fps: Option<f32>,
        recompiled: bool,
        fast: bool,
        jobs: &[Job],
        waiting: usize,
        saved: Option<&str>,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let ctx = ui.ctx().clone();
        if let Some((text, since)) = &self.toast {
            if since.elapsed() < std::time::Duration::from_millis(2500) {
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 8.0))
                    .interactable(false)
                    .show(&ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label(text);
                        });
                    });
                ctx.request_repaint();
            } else {
                self.toast = None;
            }
        }
        if fast {
            egui::Area::new(egui::Id::new("fast forward"))
                .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-8.0, 8.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(RichText::new("» fast forward").monospace());
                    });
                });
        }
        if let Some(fps) = fps {
            // clicks go thru to the screen under it
            egui::Area::new(egui::Id::new("fps")).fixed_pos(egui::pos2(8.0, 8.0)).interactable(false).show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(RichText::new(format!("{fps:.0} fps")).monospace());
                });
            });
        }
        if !self.menu_open {
            return actions;
        }
        egui::Window::new("Paused")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(&ctx, |ui| {
                ui.label(RichText::new(name).strong());
                ui.label(RichText::new(if recompiled { "running its recompiled code" } else { "interpreted" }).weak());
                ui.add_space(6.0);
                let wide = |ui: &mut egui::Ui, text: &str| ui.add_sized([200.0, 28.0], egui::Button::new(text)).clicked();
                if wide(ui, "Resume") {
                    actions.push(Action::Resume);
                }
                if wide(ui, "Reset") {
                    actions.push(Action::Reset);
                }
                if wide(ui, "Fullscreen") {
                    actions.push(Action::Fullscreen);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.slot).range(1..=9).prefix("Slot "));
                    ui.label(RichText::new(saved.unwrap_or("empty")).weak());
                });
                ui.horizontal(|ui| {
                    if ui.add_sized([98.0, 28.0], egui::Button::new("Save state")).on_hover_text("F5 saves to this slot while playing.").clicked() {
                        actions.push(Action::SaveState);
                    }
                    if ui
                        .add_enabled(saved.is_some(), egui::Button::new("Load state").min_size(Vec2::new(98.0, 28.0)))
                        .on_hover_text("F8 loads this slot while playing.")
                        .clicked()
                    {
                        actions.push(Action::LoadState);
                    }
                });
                ui.add_space(4.0);
                if wide(ui, "Settings") {
                    self.settings_open = true;
                }
                if let Some(program_id) = program_id.filter(|_| wide(ui, "Settings for this game")) {
                    self.game_settings = Some((program_id, name.to_owned()));
                }
                if wide(ui, "Cheats") {
                    self.cheats_open = true;
                }
                if wide(ui, "Multiplayer") {
                    self.multiplayer_open = true;
                }
                if ui
                    .add_sized([200.0, 28.0], egui::Button::new("Show the camera a picture"))
                    .on_hover_text("Pick a picture for the console's cameras to see, a QR code's say, in place of the computer's camera.")
                    .clicked()
                {
                    actions.push(Action::ChooseCameraPicture);
                }
                if wide(ui, "Copy info for a report") {
                    actions.push(Action::CopyReport);
                }
                if wide(ui, "Back to the library") {
                    actions.push(Action::Library);
                }
                if wide(ui, "Quit") {
                    actions.push(Action::Quit);
                }
                for job in jobs.iter().filter(|job| !job.stage().finished()) {
                    ui.separator();
                    ui.label(&job.name);
                    progress(ui, job);
                }
                if waiting > 0 {
                    ui.label(RichText::new(format!("{waiting} more waiting to recompile")).weak());
                }
            });
        actions
    }"#;

            if !src.contains(old_game_fn) {
                return Err("Impossible de localiser le corps de `Menus::game` dans `menus.rs`".into());
            }
            src = src.replace(old_game_fn, WIDE_CONSOLE_GAME_MENU_RS);
            src.push_str(WIDE_CONSOLE_WIDGET_HELPERS_RS);
            fs::write(&menus_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // 4f. `present.rs`
    let present_rs = zakuro_src.join("present.rs");
    if present_rs.exists() {
        let mut src = read_lf(&present_rs).map_err(|e| e.to_string())?;
        if !src.contains("pub fn create_secondary(") {
            let sec_fn = r#"    pub fn create_secondary(
        event_loop: &ActiveEventLoop,
        attributes: WindowAttributes,
        shared: Option<std::sync::Arc<SharedDevice>>,
    ) -> Result<(Window, Backend), String> {
        if let Some(shared_dev) = shared {
            let window = event_loop.create_window(attributes).map_err(|e| e.to_string())?;
            let size = window.inner_size();
            let mut presenter = VulkanPresenter::new_secondary(&window, (size.width.max(1), size.height.max(1)), shared_dev)
                .map_err(|e| e.to_string())?;
            presenter.set_layout(ScreenLayout::BottomOnly);
            return Ok((window, Backend::Vulkan(presenter)));
        }
        let (window, mut backend) = create_opengl(event_loop, attributes)?;
        backend.set_layout(ScreenLayout::BottomOnly);
        Ok((window, backend))
    }

    pub fn create("#;
            src = src.replace("    pub fn create(", sec_fn);
            fs::write(&present_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // 4g. `lib.rs` : auto-boot si déjà recompilé, recompilation auto dès sélection/glisser-déposer de la ROM, puis lancement auto !
    let lib_rs = zakuro_src.join("lib.rs");
    if lib_rs.exists() {
        let mut src = read_lf(&lib_rs).map_err(|e| e.to_string())?;
        if !src.contains("bottom_window") {
            src = src.replace(
                "        layout: settings.layout,",
                "        layout: settings.layout,\n        last_combined_layout: if settings.layout == Screens::SeparateWindows { Screens::Stacked } else { settings.layout },",
            );
            src = src.replace(
                "        backend: None,\n        window: None,",
                "        backend: None,\n        window: None,\n        bottom_backend: None,\n        bottom_window: None,\n        bottom_cursor: (0.0, 0.0),\n        bottom_mouse_down: false,",
            );
            src = src.replace(
                "    backend: Option<Backend>,\n    window: Option<Window>,",
                "    backend: Option<Backend>,\n    window: Option<Window>,\n    bottom_backend: Option<Backend>,\n    bottom_window: Option<Window>,\n    bottom_cursor: (f32, f32),\n    bottom_mouse_down: bool,",
            );
            src = src.replace(
                "    layout: Screens,",
                "    layout: Screens,\n    last_combined_layout: Screens,",
            );

            // Toujours activer le profil matériel New 3DS pour Pokémon Saphir Alpha
            src = src.replace(
                "            new3ds: self.options.new3ds,",
                "            new3ds: true,",
            );

            // Démarrage intelligent : si une ROM a déjà été configurée, lancer tout de suite le jeu (ou reprendre la recompilation)
            let start_needle = r#"        } else if let Some(rom) = self.options.rom.clone() {
            if let Err(error) = self.play(Path::new(&rom)) {
                eprintln!("zakuro: {error}");
                std::process::exit(1);
            }
        }"#;
            let start_rep = r#"        } else if let Some(rom) = self.options.rom.clone() {
            if let Err(error) = self.play(Path::new(&rom)) {
                eprintln!("zakuro: {error}");
                std::process::exit(1);
            }
        } else if let Some(rom) = self.settings.blueorb_rom.clone().filter(|p| p.is_file()) {
            self.accept_rom_file(rom);
        }"#;
            src = src.replace(start_needle, start_rep);

            let set_layout_needle = "    fn set_layout(&mut self, layout: Screens) {";
            let set_layout_rep = r#"    fn accept_rom_file(&mut self, path: PathBuf) {
        let title = match zakuro_fs::Title::load(&path) {
            Ok(t) => t,
            Err(err) => {
                self.menus.message = Some(format!(
                    "Could not open ROM ({err}).\n\nPlease select a decrypted Nintendo 3DS ROM (.3ds, .cci, or .cxi)."
                ));
                return;
            }
        };
        let program_id = title.program_id();
        self.settings.blueorb_rom = Some(path.clone());
        if let Some(parent) = path.parent() {
            if self.settings.games.is_none() {
                self.settings.games = Some(parent.to_path_buf());
            }
        }
        let tid_hex = format!("{program_id:016X}");
        if (program_id == 0x0004_0000_0011_C500 || program_id == 0x0004_0000_0011_C400)
            && !self.settings.enhancements.contains_key(&tid_hex)
        {
            self.settings.enhancements.insert(tid_hex, vec!["No Outlines (HD)".to_owned()]);
        }
        self.settings.save();

        let already_recompiled = self.linked.is_some_and(|l| l.program_id() == program_id)
            || zakuro_core::recompiled::installed(program_id).is_some();

        if already_recompiled {
            self.apply_official_rom_icon(&path);
            if let Err(error) = self.play(&path) {
                self.menus.message = Some(error);
            }
        } else {
            self.menus.menu_open = false;
            self.game = None;
            self.recompile(vec![Wanted { program_id, path }], true);
        }
    }

    fn apply_official_rom_icon(&self, rom_path: &Path) {
        let Ok(title) = zakuro_fs::Title::load(rom_path) else { return };
        let Some(smdh) = title.exefs_file("icon").and_then(library::Smdh::parse) else { return };
        if smdh.icon.len() != 48 * 48 * 4 {
            return;
        }
        let rgba64 = blueorb_upscale_smdh_rgba(&smdh.icon, 64);
        if let Ok(win_icon) = winit::window::Icon::from_rgba(rgba64, 64, 64) {
            if let Some(w) = &self.window {
                w.set_window_icon(Some(win_icon.clone()));
            }
            if let Some(bw) = &self.bottom_window {
                bw.set_window_icon(Some(win_icon));
            }
        }
        let icon48 = smdh.icon;
        let game_title = if smdh.name.is_empty() {
            "Pokémon Alpha Sapphire".to_owned()
        } else {
            smdh.name
        };
        std::thread::spawn(move || {
            blueorb_persist_official_bundle_icon(&icon48, &game_title);
        });
    }

    /// Removes everything BlueOrb generated outside the app (native library, build cache, downloaded toolchain).
    /// Save files are never touched.
    fn clean_residual_files(&mut self) {
        let confirm = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Clean Up Residual Files")
            .set_description(
                "This removes the recompiled native library, the C build cache and the downloaded compiler.\n\nYour save files are kept. The game will be recompiled the next time you launch it.",
            )
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show();
        if confirm != rfd::MessageDialogResult::Ok {
            return;
        }
        if self.game.is_some() {
            self.back_to_library();
        }

        fn size_of(path: &Path) -> u64 {
            let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
            if !meta.is_dir() {
                return meta.len();
            }
            std::fs::read_dir(path)
                .map(|entries| entries.flatten().map(|e| size_of(&e.path())).sum())
                .unwrap_or(0)
        }
        let mut freed = 0u64;
        let mut remove = |path: &Path| {
            if !path.exists() {
                return;
            }
            let bytes = size_of(path);
            let gone = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
            match gone {
                Ok(()) => freed += bytes,
                Err(error) => log::warn!("could not remove {}: {error}", path.display()),
            }
        };

        let mut ids: Vec<u64> = vec![0x0004_0000_0011_C500, 0x0004_0000_0011_C400];
        if let Some(rom) = self.settings.blueorb_rom.as_ref() {
            if let Ok(title) = zakuro_fs::Title::load(rom) {
                ids.push(title.program_id());
            }
        }
        ids.extend(self.library.games.iter().map(|game| game.program_id));
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            if let Some(lib) = zakuro_core::recompiled::installed(id) {
                remove(&lib);
            }
            remove(&recomp3ds::build::cache_dir(id));
        }
        if let Some(tools) = zig::tools_dir(self.data_dir.as_deref()) {
            remove(&tools);
        }

        self.menus.toast = Some((
            format!("Removed {:.1} MB of residual files", freed as f64 / (1024.0 * 1024.0)),
            Instant::now(),
        ));
    }

    fn toggle_dual_windows(&mut self) {
        let target = if self.active.layout == Screens::SeparateWindows {
            self.last_combined_layout
        } else {
            self.last_combined_layout = self.active.layout;
            Screens::SeparateWindows
        };
        self.set_layout(target);
        self.menus.toast = Some((
            if target == Screens::SeparateWindows {
                "Dual Window: On".to_owned()
            } else {
                "Dual Window: Off".to_owned()
            },
            Instant::now(),
        ));
    }

    fn set_top_shape(&mut self, shape: settings::TopShape) {
        let own = self.program_id().and_then(|id| self.settings.per_game.get_mut(&settings::game_key(id)));
        match own.filter(|o| o.top_shape.is_some()) {
            Some(o) => o.top_shape = Some(shape),
            None => self.settings.top_shape = shape,
        }
        self.settings.save();
        self.active = self.settings.for_game(self.program_id());
        self.apply_layout();
        self.menus.toast = Some((format!("Aspect Ratio: {}", shape.name()), Instant::now()));
    }

    fn set_layout(&mut self, layout: Screens) {
        if layout != Screens::SeparateWindows {
            self.last_combined_layout = layout;
        }"#;
            src = src.replace(set_layout_needle, set_layout_rep);

            // Allow `next_recompile` to compile a single `.3ds` file directly without folder scanning
            let next_recomp_needle = r#"        if let Some(game) = self.library.games.iter().find(|game| game.path == wanted.path) {
            let update = game.update.as_ref().map(|update| update.path.as_path());
            self.jobs.push(Job::start(&game.path, update, game.program_id, &game.name, self.data_dir.as_deref(), toolchain));
        }"#;
            let next_recomp_rep = r#"        if let Some(game) = self.library.games.iter().find(|game| game.path == wanted.path) {
            let update = game.update.as_ref().map(|update| update.path.as_path());
            self.jobs.push(Job::start(&game.path, update, game.program_id, &game.name, self.data_dir.as_deref(), toolchain));
        } else if wanted.path.is_file() {
            let addon_upd = library::addons_beside(&wanted.path).and_then(|(u, _, _)| u.map(|x| x.path));
            let gname = wanted
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Pokémon Alpha Sapphire".to_owned());
            self.jobs.push(Job::start(
                &wanted.path,
                addon_upd.as_deref(),
                wanted.program_id,
                &gname,
                self.data_dir.as_deref(),
                toolchain,
            ));
        }"#;
            src = src.replace(next_recomp_needle, next_recomp_rep);

            // As soon as initial recompilation finishes (`Stage::Done`), apply the official Alpha Sapphire icon and auto-launch!
            let poll_done_needle = "                Stage::Done => {\n                    self.library.recompiled(program_id);";
            let poll_done_rep = r#"                Stage::Done => {
                    self.library.recompiled(program_id);
                    if self.game.is_none() {
                        if let Some(rom) = self.settings.blueorb_rom.clone().filter(|p| p.is_file()) {
                            self.apply_official_rom_icon(&rom);
                            if let Err(error) = self.play(&rom) {
                                self.menus.message = Some(error);
                            }
                            continue;
                        }
                    }"#;
            src = src.replace(poll_done_needle, poll_done_rep);

            let win_ev_needle = "    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {";
            let win_ev_rep = r#"    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.bottom_window.as_ref().is_some_and(|w| w.id() == id) {
            match event {
                WindowEvent::CloseRequested => {
                    self.bottom_mouse_down = false;
                    self.keyboard.touch(None);
                    let restore = self.last_combined_layout;
                    self.set_layout(restore);
                }
                WindowEvent::Focused(false) => {
                    self.bottom_mouse_down = false;
                    self.keyboard.release();
                }
                WindowEvent::Resized(size) => {
                    if let Some(bot) = &mut self.bottom_backend {
                        bot.resize(size.width, size.height);
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => self.key(event, false),
                WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                    self.bottom_mouse_down = state == ElementState::Pressed && self.game.is_some() && !self.menus.menu_open;
                    self.touch_bottom_window();
                }
                WindowEvent::CursorMoved { position, .. } => {
                    self.bottom_cursor = (position.x as f32, position.y as f32);
                    self.touch_bottom_window();
                }
                _ => {}
            }
            return;
        }"#;
            src = src.replace(win_ev_needle, win_ev_rep);

            // Glisser-déposer direct d'un fichier .3ds sur la fenêtre principale !
            src = src.replace(
                "            WindowEvent::RedrawRequested => self.step(event_loop),",
                "            WindowEvent::DroppedFile(path) => {\n                if self.game.is_none() || self.menus.menu_open {\n                    self.accept_rom_file(path);\n                }\n            }\n            WindowEvent::RedrawRequested => self.step(event_loop),",
            );

            let key_needle = "                KeyCode::F9 => self.set_layout(self.active.layout.next()),";
            let key_rep = r#"                KeyCode::F6 => {
                    let shapes = settings::TopShape::ALL;
                    let idx = shapes.iter().position(|&s| s == self.active.top_shape).unwrap_or(0);
                    self.set_top_shape(shapes[(idx + 1) % shapes.len()]);
                }
                KeyCode::F7 => self.toggle_dual_windows(),
                KeyCode::F9 => self.set_layout(self.active.layout.next()),"#;
            src = src.replace(key_needle, key_rep);

            let touch_needle = "    fn touch(&mut self) {\n        let Some(window) = &self.window else { return };";
            let touch_rep = r#"    fn touch_bottom_window(&mut self) {
        let Some(window) = &self.bottom_window else { return };
        let size = window.inner_size();
        let (_, bottom) = layout(size.width, size.height, zakuro_gpu::ScreenLayout::BottomOnly, self.orientation.screen_orientation(), 400.0, self.active.integer_scale);
        let Some(bottom) = bottom else {
            self.keyboard.touch(None);
            return;
        };
        let x = self.bottom_cursor.0 - bottom.x;
        let y = self.bottom_cursor.1 - bottom.y;
        let inside = x >= 0.0 && y >= 0.0 && x < bottom.width && y < bottom.height;
        self.keyboard.touch((self.bottom_mouse_down && inside).then(|| {
            let (u, v) = (x / bottom.width, y / bottom.height);
            let (u, v) = match self.orientation {
                Orientation::North => (u, v),
                Orientation::East => (v, 1.0 - u),
                Orientation::South => (1.0 - u, 1.0 - v),
                Orientation::West => (1.0 - v, u),
            };
            (((u * 320.0) as u16).min(319), ((v * 240.0) as u16).min(239))
        }));
    }

    fn touch(&mut self) {
        if self.layout == Screens::SeparateWindows {
            return;
        }
        let Some(window) = &self.window else { return };"#;
            src = src.replace(touch_needle, touch_rep);

            let apply_needle = "    fn apply_layout(&mut self) {\n        if let Some(backend) = &mut self.backend {";
            let apply_rep = r#"    fn apply_layout(&mut self) {
        zakuro_gpu::set_top_aspect_scale(self.active.top_shape.width());
        if let Some(bot) = &mut self.bottom_backend {
            bot.set_scaling(self.active.filter.screens(), self.active.integer_scale);
            bot.set_orientation(self.orientation.screen_orientation());
        }
        if let Some(backend) = &mut self.backend {"#;
            src = src.replace(apply_needle, apply_rep);

            src = src.replace(
                "backend.set_top_width(self.active.top_shape.width());",
                "zakuro_gpu::set_top_aspect_scale(self.active.top_shape.width());\n                backend.set_top_width(self.active.top_shape.width());",
            );

            src = src.replace(
                "            Action::Quit => event_loop.exit(),",
                r#"            Action::PickRomFile => {
                let picked = rfd::FileDialog::new()
                    .set_title("Select Decrypted Nintendo 3DS ROM (.3ds)")
                    .add_filter("Nintendo 3DS ROM (.3ds, .cci, .cxi)", &["3ds", "cci", "cxi"])
                    .pick_file();
                if let Some(file) = picked {
                    self.accept_rom_file(file);
                }
            }
            Action::CleanResiduals => self.clean_residual_files(),
            Action::InstallToApplications => {
                #[cfg(target_os = "macos")]
                blueorb_move_bundle_to_applications_now();
            }
            Action::ToggleDualWindows => self.toggle_dual_windows(),
            Action::SetLayout(layout) => self.set_layout(layout),
            Action::SetTopShape(shape) => self.set_top_shape(shape),
            Action::Quit => event_loop.exit(),"#,
            );

            src = src.replace(
                "Some(game) => format!(\"Zakuro - {} - {}\", game.name, game.system.status_line()),\n                    None => \"Zakuro\".to_owned(),",
                "Some(game) => game.name.clone(),\n                    None => \"BlueOrb\".to_owned(),",
            );
            src = src.replace(
                "Window::default_attributes().with_title(\"Zakuro\")",
                "Window::default_attributes().with_title(\"BlueOrb\").with_window_icon(blueorb_initial_window_icon(self.settings.blueorb_rom.as_deref()))",
            );

            src = src.replace(
                "        let cheats = self.game.as_ref().filter(|_| self.menus.cheats_open).map(|game| {",
                "        let cheats = self.game.as_ref().filter(|_| self.menus.cheats_open || self.menus.menu_open).map(|game| {",
            );
            src = src.replace(
                "        let (menus, library, settings, jobs, waiting) = (&mut self.menus, &self.library, &mut self.settings, &self.jobs, &self.waiting);",
                "        let (active_layout, active_top_shape) = (self.active.layout, self.active.top_shape);\n        let (menus, library, settings, jobs, waiting) = (&mut self.menus, &self.library, &mut self.settings, &self.jobs, &self.waiting);",
            );
            src = src.replace(
                "menus.game(ui, name, program_id, show_fps.then_some(*fps), *recompiled, fast, jobs, waiting.len(), saved.as_deref())",
                "menus.game(ui, name, program_id, show_fps.then_some(*fps), *recompiled, fast, jobs, waiting.len(), saved.as_deref(), settings, cheats.as_deref(), active_layout, active_top_shape)",
            );

            let present_needle = "    fn present(&mut self, event_loop: &ActiveEventLoop) {\n        let overlay = self.interface(event_loop);";
            let present_rep = r#"    fn sync_bottom_window(&mut self, event_loop: &ActiveEventLoop) {
        let wants_second = self.layout == Screens::SeparateWindows && self.game.is_some();
        if wants_second && self.bottom_window.is_none() {
            let bot_scale = self.scale.clamp(2, 4);
            let bot_size = winit::dpi::LogicalSize::new(320 * bot_scale, 240 * bot_scale);
            let attrs = Window::default_attributes()
                .with_title("BlueOrb — Touch Screen")
                .with_window_icon(blueorb_initial_window_icon(self.settings.blueorb_rom.as_deref()))
                .with_inner_size(bot_size);
            let shared = self.backend.as_ref().and_then(Backend::shared_device);
            match Backend::create_secondary(event_loop, attrs, shared) {
                Ok((win, mut bot_backend)) => {
                    bot_backend.set_scaling(self.active.filter.screens(), self.active.integer_scale);
                    bot_backend.set_orientation(self.orientation.screen_orientation());
                    if let Some(main_win) = &self.window {
                        if let Ok(pos) = main_win.outer_position() {
                            let main_sz = main_win.outer_size();
                            win.set_outer_position(winit::dpi::PhysicalPosition::new(
                                pos.x + (main_sz.width as i32) + 16,
                                pos.y,
                            ));
                        }
                    }
                    self.bottom_window = Some(win);
                    self.bottom_backend = Some(bot_backend);
                }
                Err(err) => log::warn!("could not open secondary touch window: {err}"),
            }
        } else if !wants_second && self.bottom_window.is_some() {
            self.bottom_mouse_down = false;
            self.bottom_backend = None;
            self.bottom_window = None;
        }
    }

    fn present(&mut self, event_loop: &ActiveEventLoop) {
        let overlay = self.interface(event_loop);
        self.sync_bottom_window(event_loop);"#;
            src = src.replace(present_needle, present_rep);

            let present_end_needle = r#"        match result {
            Ok(()) | Err(PresentError::OutOfDate) => {}
            Err(error) => {
                log::error!("presentation failed: {error}");
                event_loop.exit();
            }
        }
    }"#;
            let present_end_rep = r#"        match result {
            Ok(()) | Err(PresentError::OutOfDate) => {}
            Err(error) => {
                log::error!("presentation failed: {error}");
                event_loop.exit();
            }
        }
        if let Some(bot_backend) = &mut self.bottom_backend {
            let empty_px: Vec<u8> = Vec::new();
            let empty_overlay = Overlay::default();
            let _ = bot_backend.present(
                ScreenImage { width: 400, height: 240, pixels: &empty_px, gpu: None },
                ScreenImage { width: bottom_width, height: bottom_height, pixels: &bottom, gpu: bottom_gpu },
                ScreenImage { width: 400, height: 240, pixels: &empty_px, gpu: None },
                &empty_overlay,
            );
        }
    }"#;
            src = src.replace(present_end_needle, present_end_rep);
            src.push_str(OFFICIAL_ICON_UPDATER_RS);

            fs::write(&lib_rs, src).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

/// Writes `settings.toml` (`frame_interpolation = false` and `language = "english"` by default).
pub fn write_remaster_settings_toml(
    settings_path: &Path,
    title_id: &str,
    internal_res: u32,
    no_outlines: bool,
    unlock_turbo_60: bool,
    language_code: u32,
    top_aspect: &str,
    dual_windows: bool,
    force_overwrite: bool,
) -> Result<(), String> {
    if settings_path.exists() && !force_overwrite {
        return Ok(());
    }
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let lang_name = match language_code {
        0 => "japanese",
        1 => "english",
        2 => "french",
        3 => "german",
        4 => "italian",
        5 => "spanish",
        _ => "english",
    };

    let layout_str = if dual_windows {
        "separate_windows"
    } else {
        "stacked"
    };

    let mut active_enhancements = Vec::new();
    if no_outlines {
        active_enhancements.push("\"No Outlines (HD)\"");
    }
    if unlock_turbo_60 {
        active_enhancements.push("\"60 FPS\"");
    }
    let enh_list = active_enhancements.join(", ");

    let toml_text = format!(
        r#"# Configuration BlueOrb (zéro clignotement 60 Hz par défaut)
language = "{lang}"
renderer = "vulkan"
hardware_rasterizer = true
resolution = {res}
texture_packs = true
frame_interpolation = false
recompiled = true
scale = 2
layout = "{layout}"
orientation = "north"
filter = "sharp"
integer_scale = false
top_shape = "{top_shape}"
stereo = "off"
depth_3d = 50
swap_eyes = false
volume = 1.0
mute = false
show_fps = false

[enhancements]
{tid} = [{enh}]
"#,
        lang = lang_name,
        res = internal_res.clamp(1, 10),
        layout = layout_str,
        top_shape = top_aspect,
        tid = title_id,
        enh = enh_list,
    );

    fs::write(settings_path, toml_text).map_err(|e| e.to_string())
}

const CONSOLE_THEME_FN_RS: &str = r#"
// BLUEORB_CONSOLE_THEME
fn apply_blueorb_console_theme(ctx: &egui::Context) {
    use egui::{Color32, CornerRadius, CursorIcon, FontFamily, FontId, Shadow, Stroke, TextStyle, Theme};
    ctx.set_theme(Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(TextStyle::Small, FontId::new(12.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Body, FontId::new(13.5, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Button, FontId::new(13.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Heading, FontId::new(17.5, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace));

        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.window_margin = egui::Margin::same(14);

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.window_fill = Color32::from_rgb(24, 24, 27);
        v.panel_fill = Color32::from_rgb(24, 24, 27);
        v.faint_bg_color = Color32::from_rgb(31, 31, 35);
        v.extreme_bg_color = Color32::from_rgb(17, 17, 20);
        v.code_bg_color = Color32::from_rgb(17, 17, 20);
        v.window_corner_radius = CornerRadius::same(5);
        v.menu_corner_radius = CornerRadius::same(4);
        v.window_stroke = Stroke::new(1.0, Color32::from_rgb(54, 54, 61));
        v.window_shadow = Shadow {
            offset: [0, 8],
            blur: 20,
            spread: 0,
            color: Color32::from_black_alpha(140),
        };
        v.popup_shadow = Shadow {
            offset: [0, 4],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(120),
        };
        v.slider_trailing_fill = true;
        v.interact_cursor = Some(CursorIcon::PointingHand);
        v.selection.bg_fill = Color32::from_rgb(56, 98, 154);
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);

        v.widgets.noninteractive.bg_fill = Color32::from_rgb(30, 30, 34);
        v.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(30, 30, 34);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(50, 50, 57));
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(228, 228, 232));
        v.widgets.noninteractive.corner_radius = CornerRadius::same(4);

        v.widgets.inactive.bg_fill = Color32::from_rgb(38, 38, 43);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(38, 38, 43);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(60, 60, 68));
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(228, 228, 232));
        v.widgets.inactive.corner_radius = CornerRadius::same(4);

        v.widgets.hovered.bg_fill = Color32::from_rgb(48, 48, 55);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(48, 48, 55);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(82, 82, 92));
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        v.widgets.hovered.corner_radius = CornerRadius::same(4);

        v.widgets.active.bg_fill = Color32::from_rgb(56, 98, 154);
        v.widgets.active.weak_bg_fill = Color32::from_rgb(56, 98, 154);
        v.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(88, 134, 196));
        v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        v.widgets.active.corner_radius = CornerRadius::same(4);

        v.widgets.open = v.widgets.hovered;
    });
}
"#;

const ONBOARDING_LIBRARY_FN_RS: &str = r#"    /// Clean, utilitarian native desktop launcher with royalty-free Sapphire Ocean vista:
    /// - First launch: drop or select a decrypted .3ds ROM -> automatic native AOT build -> launches straight into the game
    /// - Subsequent launches: boots directly into the game
    pub fn library(&mut self, ui: &mut egui::Ui, library: &Library, settings: &Settings, jobs: &[Job], waiting: &[Wanted]) -> Vec<Action> {
        let mut actions = Vec::new();
        let _ = (
            self.background.get(ui.ctx(), None),
            idle(library.games.iter().map(Game::wanted), jobs, waiting),
            ICON_SIZE,
        );

        let active_job = jobs.iter().find(|j| !j.stage().finished());
        let failed_job = jobs.iter().rev().find_map(|j| match j.stage() {
            Stage::Failed(err) if err != "cancelled" => Some((j.program_id, err)),
            _ => None,
        });
        let hovering_file = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let show_install_apps = can_install_to_applications();

        // Top toolbar
        egui::Panel::top("blueorb_launcher_topbar")
            .resizable(false)
            .exact_size(40.0)
            .show_separator_line(true)
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(12, 16, 26, 224))
                    .inner_margin(egui::Margin::symmetric(14, 7)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (icon_rect, _) = ui.allocate_exact_size(Vec2::new(18.0, 18.0), egui::Sense::hover());
                    paint_diveball_emblem(ui.painter(), icon_rect.center(), 8.5);
                    ui.label(RichText::new("BlueOrb").size(13.5).strong().color(Color32::from_rgb(236, 240, 248)));
                    ui.label(RichText::new("·  Native AOT Port").size(12.0).color(Color32::from_rgb(138, 148, 168)));

                    let mut buttons_w = 96.0 + 8.0 + 96.0;
                    if show_install_apps {
                        buttons_w += 158.0 + 8.0;
                    }
                    let rem = (ui.available_width() - buttons_w).max(0.0);
                    ui.add_space(rem);
                    if show_install_apps && console_secondary_button(ui, 158.0, 25.0, "Install to /Applications", true) {
                        actions.push(Action::InstallToApplications);
                    }
                    if console_secondary_button(ui, 96.0, 25.0, "Clean Cache", active_job.is_none()) {
                        actions.push(Action::CleanResiduals);
                    }
                    if console_secondary_button(ui, 96.0, 25.0, "Open ROM…", active_job.is_none()) {
                        actions.push(Action::PickRomFile);
                    }
                });
            });

        // Bottom status bar
        let status_left = if let Some(job) = active_job {
            match job.stage() {
                Stage::Downloading(p) => format!("Downloading Zig C compiler ({:.0}%)", p * 100.0),
                Stage::Generating => "Translating ARM11 / CRO code to C...".to_owned(),
                Stage::Compiling { done, total } => format!("Compiling object files ({done}/{total})"),
                Stage::Installing => "Linking native library...".to_owned(),
                Stage::Done => "Launching...".to_owned(),
                Stage::Failed(e) => format!("Build failed: {e}"),
            }
        } else if let Some((_, ref err)) = failed_job {
            format!("Error: {err}")
        } else if let Some(rom) = settings.blueorb_rom.as_ref().filter(|p| p.is_file()) {
            rom.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "ROM ready".to_owned())
        } else {
            "Waiting for .3ds ROM".to_owned()
        };

        egui::Panel::bottom("blueorb_launcher_statusbar")
            .resizable(false)
            .exact_size(24.0)
            .show_separator_line(true)
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(10, 14, 22, 232))
                    .inner_margin(egui::Margin::symmetric(14, 3)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&status_left).size(11.5).monospace().color(Color32::from_rgb(154, 164, 180)));
                    let right_lbl = format!("{}  ·  Vulkan", std::env::consts::ARCH);
                    let rem = (ui.available_width() - 120.0).max(0.0);
                    ui.add_space(rem);
                    ui.label(RichText::new(right_lbl).size(11.5).monospace().color(Color32::from_rgb(120, 130, 148)));
                });
            });

        if let Some((text, since)) = &self.toast {
            if since.elapsed() < std::time::Duration::from_millis(2600) {
                let ctx = ui.ctx().clone();
                egui::Area::new(egui::Id::new("launcher_toast"))
                    .order(egui::Order::Foreground)
                    .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 50.0))
                    .interactable(false)
                    .show(&ctx, |ui| {
                        egui::Frame::NONE
                            .fill(Color32::from_rgba_unmultiplied(14, 19, 30, 236))
                            .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 175, 245, 72)))
                            .corner_radius(egui::CornerRadius::same(5))
                            .inner_margin(egui::Margin::symmetric(12, 6))
                            .show(ui, |ui| {
                                ui.label(RichText::new(text).size(13.0).color(Color32::from_rgb(236, 242, 250)));
                            });
                    });
                ctx.request_repaint();
            } else {
                self.toast = None;
            }
        }

        // Main workspace with cached royalty-free Sapphire Ocean scenic vista
        let ws_frame = egui::Frame::NONE
            .fill(Color32::TRANSPARENT)
            .inner_margin(egui::Margin::symmetric(24, 20));

        egui::CentralPanel::default().frame(ws_frame).show(ui, |ui| {
            let full_rect = ui.max_rect();
            let ocean_tex = self.icons.entry(PathBuf::from("__blueorb_ocean_bg__")).or_insert_with(|| {
                let img = blueorb_generate_ocean_scenery(640, 360);
                ui.ctx().load_texture("blueorb_ocean_bg", img, egui::TextureOptions::LINEAR)
            });
            let uv = cover(full_rect.size(), [640, 360]);
            ui.painter().image(ocean_tex.id(), full_rect, uv, Color32::WHITE);

            let avail_w = ui.available_width();
            let avail_h = ui.available_height();
            let box_w = (avail_w - 8.0).clamp(260.0, 470.0);

            if let Some(job) = active_job {
                ui.ctx().request_repaint();
                let top_pad = ((avail_h - 150.0) * 0.45).max(8.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(top_pad);
                    egui::Frame::NONE
                        .fill(Color32::from_rgba_unmultiplied(15, 20, 30, 230))
                        .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 175, 245, 56)))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(egui::Margin::same(18))
                        .show(ui, |ui| {
                            let cw = (box_w - 38.0).max(160.0);
                            ui.set_min_width(cw);
                            ui.set_max_width(cw);

                            let stage = job.stage();
                            let mins = job.elapsed().as_secs() / 60;
                            let secs = job.elapsed().as_secs() % 60;

                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&job.name).size(14.0).strong().color(Color32::from_rgb(238, 242, 248)));
                                let rem = (ui.available_width() - 48.0).max(0.0);
                                ui.add_space(rem);
                                ui.label(RichText::new(format!("{mins:02}:{secs:02}")).size(12.0).monospace().color(Color32::from_rgb(154, 166, 186)));
                            });

                            ui.add_space(8.0);
                            let (bar_rect, _) = ui.allocate_exact_size(Vec2::new(cw, 7.0), egui::Sense::hover());
                            if ui.is_rect_visible(bar_rect) {
                                ui.painter().rect_filled(bar_rect, egui::CornerRadius::same(2), Color32::from_rgb(12, 16, 24));
                                let fill_w = (cw * stage.fraction().clamp(0.0, 1.0)).max(2.0);
                                let fill_rect = egui::Rect::from_min_size(bar_rect.min, Vec2::new(fill_w, bar_rect.height()));
                                ui.painter().rect_filled(fill_rect, egui::CornerRadius::same(2), Color32::from_rgb(76, 148, 232));
                            }

                            ui.add_space(10.0);
                            let step_txt = match &stage {
                                Stage::Downloading(p) => format!("Downloading portable C toolchain ({:.0}%)", p * 100.0),
                                Stage::Generating => "Generating C from ARM11 & CRO modules".to_owned(),
                                Stage::Compiling { done, total } => format!("Compiling C units ({done} / {total})"),
                                Stage::Installing => "Linking shared library".to_owned(),
                                Stage::Done => "Starting game...".to_owned(),
                                Stage::Failed(e) => format!("Failed: {e}"),
                            };
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(step_txt).size(12.0).monospace().color(Color32::from_rgb(172, 182, 198)));
                                let rem = (ui.available_width() - 78.0).max(0.0);
                                ui.add_space(rem);
                                if console_secondary_button(ui, 76.0, 26.0, "Cancel", true) {
                                    actions.push(Action::CancelRecompile(job.program_id));
                                }
                            });
                        });
                });
            } else {
                let top_pad = ((avail_h - 210.0) * 0.44).max(8.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(top_pad);

                    let border = if hovering_file {
                        egui::Stroke::new(1.5, Color32::from_rgb(104, 178, 255))
                    } else {
                        egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 175, 245, 52))
                    };
                    let fill = if hovering_file {
                        Color32::from_rgba_unmultiplied(20, 32, 50, 236)
                    } else {
                        Color32::from_rgba_unmultiplied(15, 20, 30, 224)
                    };

                    egui::Frame::NONE
                        .fill(fill)
                        .stroke(border)
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(24, 24))
                        .show(ui, |ui| {
                            let cw = (box_w - 50.0).max(180.0);
                            ui.set_min_width(cw);
                            ui.set_max_width(cw);

                            ui.vertical_centered(|ui| {
                                let (emb_rect, _) = ui.allocate_exact_size(Vec2::new(46.0, 46.0), egui::Sense::hover());
                                paint_diveball_emblem(ui.painter(), emb_rect.center(), 21.0);

                                ui.add_space(8.0);
                                if let Some(saved_rom) = settings.blueorb_rom.as_ref().filter(|p| p.is_file()) {
                                    let fname = saved_rom.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                    ui.label(RichText::new(&fname).size(13.5).strong().color(Color32::from_rgb(238, 242, 248)));
                                    ui.add_space(12.0);
                                    ui.horizontal(|ui| {
                                        let pad_l = ((cw - 218.0) * 0.5).max(0.0);
                                        ui.add_space(pad_l);
                                        if console_primary_button(ui, 104.0, 30.0, "Launch") {
                                            actions.push(Action::Play(saved_rom.clone()));
                                        }
                                        if console_secondary_button(ui, 104.0, 30.0, "Change ROM…", true) {
                                            actions.push(Action::PickRomFile);
                                        }
                                    });
                                } else {
                                    ui.label(RichText::new("BlueOrb Launcher").size(15.0).strong().color(Color32::from_rgb(238, 242, 248)));
                                    ui.add_space(3.0);
                                    ui.label(
                                        RichText::new("Drop a decrypted .3ds / .cci ROM here or select one from disk.")
                                            .size(12.5)
                                            .color(Color32::from_rgb(162, 172, 190)),
                                    );
                                    ui.add_space(14.0);
                                    if console_primary_button(ui, 146.0, 32.0, "Select ROM…") {
                                        actions.push(Action::PickRomFile);
                                    }
                                }

                                if let Some((_, ref err)) = failed_job {
                                    ui.add_space(10.0);
                                    ui.label(RichText::new(err).size(12.0).color(Color32::from_rgb(248, 138, 138)));
                                }
                            });
                        });
                });
            }
        });
        actions
    }

"#;

const WIDE_CONSOLE_GAME_MENU_RS: &str = r#"    /// BLUEORB_WIDE_CONSOLE_MENU: Clean native desktop pause overlay (English by default)
    #[allow(clippy::too_many_arguments)]
    pub fn game(
        &mut self,
        ui: &mut egui::Ui,
        name: &str,
        program_id: Option<u64>,
        fps: Option<f32>,
        recompiled: bool,
        fast: bool,
        jobs: &[Job],
        waiting: usize,
        saved: Option<&str>,
        settings: &mut Settings,
        cheats: Option<&[(String, bool, String, bool)]>,
        active_layout: Screens,
        active_top_shape: TopShape,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let before_settings = settings.clone();
        let ctx = ui.ctx().clone();

        let hud_frame = || {
            egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(17, 17, 20, 224))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(54, 54, 62)))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(10, 5))
        };

        if let Some((text, since)) = &self.toast {
            if since.elapsed() < std::time::Duration::from_millis(2200) {
                egui::Area::new(egui::Id::new("toast"))
                    .order(egui::Order::Foreground)
                    .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 12.0))
                    .interactable(false)
                    .show(&ctx, |ui| {
                        hud_frame().show(ui, |ui| {
                            ui.label(RichText::new(text).size(13.0).color(Color32::from_rgb(236, 236, 240)));
                        });
                    });
                ctx.request_repaint();
            } else {
                self.toast = None;
            }
        }
        if fast && !self.menu_open {
            egui::Area::new(egui::Id::new("fast forward"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-10.0, 10.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    hud_frame().show(ui, |ui| {
                        ui.label(RichText::new("Fast Forward").size(12.0).monospace());
                    });
                });
        }
        if let Some(fps) = fps.filter(|_| !self.menu_open) {
            egui::Area::new(egui::Id::new("fps"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(10.0, 10.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    hud_frame().show(ui, |ui| {
                        ui.label(RichText::new(format!("{fps:.0} FPS")).size(12.0).monospace());
                    });
                });
        }
        if !self.menu_open {
            return actions;
        }

        let nav_tabs: [(u8, &str); 5] = [
            (0, "Game"),
            (1, "Display"),
            (2, "Graphics & Audio"),
            (3, "Controls"),
            (4, "System & Cheats"),
        ];

        let total_w = ui.available_width();
        let wide_mode = total_w >= 540.0;

        if wide_mode {
            let sidebar_w = 196.0;
            let sidebar_frame = egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(18, 18, 21, 248))
                .inner_margin(egui::Margin::symmetric(14, 18));

            egui::Panel::left("blueorb_pause_sidebar")
                .resizable(false)
                .exact_size(sidebar_w)
                .show_separator_line(true)
                .frame(sidebar_frame)
                .show(ui, |ui| {
                    let item_w = ui.available_width();

                    egui::Panel::bottom("blueorb_pause_sidebar_footer")
                        .resizable(false)
                        .show_separator_line(false)
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 0, right: 0, top: 8, bottom: 0 }))
                        .show(ui, |ui| {
                            if console_primary_button(ui, item_w, 34.0, "Resume") {
                                actions.push(Action::Resume);
                            }
                            ui.add_space(6.0);
                            if console_danger_button(ui, item_w, 32.0, "Quit") {
                                actions.push(Action::Quit);
                            }
                        });

                    ui.label(RichText::new(name).size(14.5).strong().color(Color32::from_rgb(236, 236, 240)));
                    ui.label(
                        RichText::new(if recompiled { "Native AOT" } else { "Interpreter" })
                            .size(11.5)
                            .monospace()
                            .color(Color32::from_rgb(130, 130, 140)),
                    );
                    ui.add_space(14.0);

                    ui.spacing_mut().item_spacing.y = 4.0;
                    for (idx, title) in nav_tabs {
                        if console_nav_button(ui, item_w, title, self.tab == idx) {
                            self.tab = idx;
                        }
                    }
                });
        } else {
            let top_frame = egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(18, 18, 21, 248))
                .inner_margin(egui::Margin::symmetric(10, 8));

            egui::Panel::top("blueorb_pause_topbar")
                .resizable(false)
                .show_separator_line(true)
                .frame(top_frame)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (idx, title) in nav_tabs {
                            let sel = self.tab == idx;
                            let btn = egui::Button::new(
                                RichText::new(title)
                                    .size(12.5)
                                    .color(if sel { Color32::WHITE } else { Color32::from_rgb(170, 170, 180) }),
                            )
                            .fill(if sel { Color32::from_rgb(56, 98, 154) } else { Color32::from_rgb(30, 30, 35) });
                            if ui.add(btn).clicked() {
                                self.tab = idx;
                            }
                        }
                    });
                });
        }

        let central_margin_x: i8 = if wide_mode { 24 } else { 14 };
        let central_frame = egui::Frame::NONE
            .fill(Color32::from_rgba_unmultiplied(24, 24, 27, 245))
            .inner_margin(egui::Margin::symmetric(central_margin_x, 18));

        egui::CentralPanel::default().frame(central_frame).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("blueorb_pause_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let body_w = (ui.available_width() - 10.0).clamp(240.0, 680.0);
                    ui.set_max_width(body_w);
                    self.draw_console_tab_body(
                        ui,
                        body_w,
                        program_id,
                        jobs,
                        waiting,
                        saved,
                        settings,
                        cheats,
                        active_layout,
                        active_top_shape,
                        &mut actions,
                    );
                    ui.add_space(12.0);
                });
        });

        if *settings != before_settings {
            if let Some(pid) = program_id {
                if let Some(own) = settings.per_game.get_mut(&game_key(pid)) {
                    if own.resolution.is_some() {
                        own.resolution = Some(settings.resolution);
                    }
                    if own.frame_interpolation.is_some() {
                        own.frame_interpolation = Some(settings.frame_interpolation);
                    }
                    if own.scale.is_some() {
                        own.scale = Some(settings.scale);
                    }
                    if own.filter.is_some() {
                        own.filter = Some(settings.filter);
                    }
                    if own.integer_scale.is_some() {
                        own.integer_scale = Some(settings.integer_scale);
                    }
                    if own.language.is_some() {
                        own.language = Some(settings.language);
                    }
                }
            }
            actions.push(Action::Settings);
        }
        actions
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_console_tab_body(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        program_id: Option<u64>,
        jobs: &[Job],
        waiting: usize,
        saved: Option<&str>,
        settings: &mut Settings,
        cheats: Option<&[(String, bool, String, bool)]>,
        active_layout: Screens,
        active_top_shape: TopShape,
        actions: &mut Vec<Action>,
    ) {
        match self.tab {
            0 => {
                // TAB 0: GAME
                ui.label(RichText::new("Paused").size(18.0).strong().color(Color32::from_rgb(238, 238, 242)));
                let layout_desc = match active_layout {
                    Screens::Stacked => "Stacked",
                    Screens::LargeTop => "Large Top",
                    Screens::SideBySide => "Side by Side",
                    Screens::SeparateWindows => "Dual Window",
                    Screens::TopOnly => "Top Only",
                    Screens::BottomOnly => "Bottom Only",
                };
                let ratio_desc = match active_top_shape {
                    TopShape::Console => "5:3",
                    TopShape::Wide16x9 => "16:9",
                    TopShape::Wide21x9 => "21:9",
                    TopShape::Wide32x9 => "32:9",
                };
                ui.label(
                    RichText::new(format!("{ratio_desc}  ·  {layout_desc}"))
                        .size(12.5)
                        .monospace()
                        .color(Color32::from_rgb(136, 136, 146)),
                );
                ui.add_space(14.0);

                let third = (width - 16.0) / 3.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if console_primary_button(ui, third, 34.0, "Resume (Esc)") {
                        actions.push(Action::Resume);
                    }
                    if console_secondary_button(ui, third, 34.0, "Fullscreen (F11)", true) {
                        actions.push(Action::Fullscreen);
                    }
                    if console_secondary_button(ui, third, 34.0, "Reset", true) {
                        actions.push(Action::Reset);
                    }
                });

                ui.add_space(14.0);

                console_card(ui, width, |ui, cw| {
                    let state_info = saved.unwrap_or("Empty");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Save States").size(14.0).strong().color(Color32::from_rgb(236, 236, 240)));
                        ui.label(RichText::new(format!("·  Slot {} ({state_info})", self.slot)).size(12.5).color(Color32::from_rgb(140, 140, 150)));
                    });
                    ui.add_space(8.0);

                    let slot_opts: [(u8, &str); 9] = [
                        (1, "1"), (2, "2"), (3, "3"), (4, "4"), (5, "5"), (6, "6"), (7, "7"), (8, "8"), (9, "9"),
                    ];
                    if let Some(s) = console_segmented(ui, cw, 32.0, self.slot, &slot_opts, "slots_seg") {
                        self.slot = s;
                    }

                    ui.add_space(10.0);
                    let half = (cw - 8.0) / 2.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if console_secondary_button(ui, half, 32.0, "Save State (F5)", true) {
                            actions.push(Action::SaveState);
                        }
                        if console_secondary_button(ui, half, 32.0, "Load State (F8)", saved.is_some()) {
                            actions.push(Action::LoadState);
                        }
                    });
                });

                ui.add_space(12.0);
                let half_bot = (width - 8.0) / 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if console_secondary_button(ui, half_bot, 32.0, "Change ROM…", true) {
                        actions.push(Action::PickRomFile);
                    }
                    if console_danger_button(ui, half_bot, 32.0, "Quit") {
                        actions.push(Action::Quit);
                    }
                });

                for job in jobs.iter().filter(|job| !job.stage().finished()) {
                    ui.add_space(8.0);
                    ui.label(&job.name);
                    progress(ui, job);
                }
                if waiting > 0 {
                    ui.label(RichText::new(format!("{waiting} queued")).weak());
                }
            }

            1 => {
                // TAB 1: DISPLAY
                ui.label(RichText::new("Display").size(18.0).strong().color(Color32::from_rgb(238, 238, 242)));
                ui.add_space(10.0);

                let card_w = (width - 10.0) / 2.0;
                let row1 = [
                    (Screens::Stacked, "Stacked (Default)", "Top screen above touch screen", 0u8),
                    (Screens::SeparateWindows, "Dual Window (F7)", "Detach touch screen into its own window", 3),
                ];
                let row2 = [
                    (Screens::LargeTop, "Large Top", "Main display with compact touch panel", 1u8),
                    (Screens::SideBySide, "Side by Side", "Horizontal dual-screen layout", 2),
                ];
                for row in [row1, row2] {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        for (lay, title, sub, icon_kind) in row {
                            let selected = active_layout == lay;
                            if console_screen_layout_card(ui, Vec2::new(card_w, 64.0), title, sub, icon_kind, selected) && !selected {
                                if lay == Screens::SeparateWindows {
                                    actions.push(Action::ToggleDualWindows);
                                } else {
                                    actions.push(Action::SetLayout(lay));
                                }
                            }
                        }
                    });
                    ui.add_space(8.0);
                }

                ui.add_space(6.0);
                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Top Screen Aspect Ratio (F6)").size(13.5).strong().color(Color32::from_rgb(232, 232, 238)));
                    ui.add_space(6.0);
                    let shape_opts = [
                        (TopShape::Console, "5:3 Native"),
                        (TopShape::Wide16x9, "16:9"),
                        (TopShape::Wide21x9, "21:9"),
                        (TopShape::Wide32x9, "32:9"),
                    ];
                    if let Some(chosen) = console_segmented(ui, cw, 32.0, active_top_shape, &shape_opts, "shape_seg") {
                        actions.push(Action::SetTopShape(chosen));
                    }

                    ui.add_space(12.0);
                    ui.label(RichText::new("Window Scale").size(13.5).strong().color(Color32::from_rgb(232, 232, 238)));
                    ui.add_space(6.0);
                    let scales = [(1u32, "1x"), (2, "2x"), (3, "3x"), (4, "4x")];
                    if let Some(s) = console_segmented(ui, cw, 32.0, settings.scale.clamp(1, 4), &scales, "scale_seg") {
                        settings.scale = s;
                    }

                    ui.add_space(12.0);
                    ui.label(RichText::new("Scaling Filter").size(13.5).strong().color(Color32::from_rgb(232, 232, 238)));
                    ui.add_space(6.0);
                    let filters = [
                        (Filter::Sharp, "Sharp"),
                        (Filter::Smooth, "Bilinear"),
                        (Filter::Pixels, "Nearest"),
                    ];
                    if let Some(f) = console_segmented(ui, cw, 32.0, settings.filter, &filters, "filter_seg") {
                        settings.filter = f;
                    }
                });
            }

            2 => {
                // TAB 2: GRAPHICS & AUDIO
                ui.label(RichText::new("Graphics & Audio").size(18.0).strong().color(Color32::from_rgb(238, 238, 242)));
                ui.add_space(10.0);

                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Internal 3D Resolution (applies on restart)").size(13.5).strong().color(Color32::from_rgb(232, 232, 238)));
                    ui.add_space(6.0);
                    let res_opts = [
                        (1u32, "1x (240p)"),
                        (3, "3x (720p)"),
                        (4, "4x (1080p)"),
                        (5, "5x (1440p)"),
                        (8, "8x (4K)"),
                    ];
                    let current_preset = match settings.resolution {
                        0..=2 => 1,
                        3 => 3,
                        4 => 4,
                        5..=6 => 5,
                        _ => 8,
                    };
                    if let Some(r) = console_segmented(ui, cw, 32.0, current_preset, &res_opts, "res_seg") {
                        settings.resolution = r;
                    }

                    if let Some(cheat_list) = cheats {
                        for (idx, (cname, enabled, notes, builtin)) in cheat_list.iter().enumerate() {
                            if !*builtin {
                                continue;
                            }
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(6.0);
                            let mut current = *enabled;
                            let title_en = if cname.contains("No Outlines") {
                                "No Outlines (HD)"
                            } else if cname == "60 FPS" {
                                "60 FPS Speedup (2x)"
                            } else {
                                cname.as_str()
                            };
                            if console_toggle_row(ui, cw, title_en, notes.as_str(), &mut current) {
                                actions.push(Action::Cheat(idx, current));
                            }
                        }
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(6.0);
                    console_toggle_row(
                        ui,
                        cw,
                        "Frame Interpolation (60 Hz)",
                        "Interpolates slow camera motion between 30 FPS frames.",
                        &mut settings.frame_interpolation,
                    );

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(6.0);
                    console_toggle_row(
                        ui,
                        cw,
                        "Custom Texture Packs",
                        "Loads replacement PNG textures from the Mods folder.",
                        &mut settings.texture_packs,
                    );
                });

                ui.add_space(12.0);

                console_card(ui, width, |ui, cw| {
                    let pct = (settings.volume * 100.0).round() as i32;
                    ui.label(RichText::new(format!("Master Volume ({pct}%)")).size(13.5).strong().color(Color32::from_rgb(232, 232, 238)));
                    ui.add_space(4.0);
                    ui.spacing_mut().slider_width = cw;
                    ui.add(egui::Slider::new(&mut settings.volume, 0.0..=1.0).show_value(false));

                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                    console_toggle_row(ui, cw, "Mute Audio", "", &mut settings.mute);

                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                    console_toggle_row(ui, cw, "Show FPS Counter", "", &mut settings.show_fps);

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.label(RichText::new("System Language (applies on restart)").size(13.5).strong().color(Color32::from_rgb(232, 232, 238)));
                    ui.add_space(6.0);
                    let lang_opts = [
                        (Language::English, "English"),
                        (Language::French, "Français"),
                        (Language::Japanese, "日本語"),
                        (Language::German, "Deutsch"),
                        (Language::Spanish, "Español"),
                        (Language::Italian, "Italiano"),
                    ];
                    if let Some(l) = console_segmented(ui, cw, 32.0, settings.language, &lang_opts, "lang_seg") {
                        settings.language = l;
                    }
                });
            }

            3 => {
                // TAB 3: CONTROLS
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Controls").size(18.0).strong().color(Color32::from_rgb(238, 238, 242)));
                    let rem = (ui.available_width() - 136.0).max(0.0);
                    ui.add_space(rem);
                    if console_secondary_button(ui, 132.0, 28.0, "Reset Defaults", true) {
                        settings.keys = Keys::default();
                        settings.pad = PadButtons::default();
                        self.rebinding = None;
                        self.rebinding_pad = None;
                    }
                });
                ui.add_space(10.0);

                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Keyboard").size(14.0).strong().color(Color32::from_rgb(236, 236, 240)));
                    ui.add_space(6.0);
                    for (i, (kname, key)) in settings.keys.all_mut().into_iter().enumerate() {
                        let waiting_this = self.rebinding == Some(i);
                        let badge = if waiting_this { "Press key…".to_owned() } else { key_name(*key) };
                        if console_binding_row(ui, cw, friendly_control_name(kname), &badge, waiting_this) {
                            self.rebinding = Some(i);
                            self.rebinding_pad = None;
                        }
                    }
                });

                ui.add_space(12.0);

                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Controller").size(14.0).strong().color(Color32::from_rgb(236, 236, 240)));
                    ui.add_space(6.0);
                    for (i, (bname, button)) in settings.pad.all_mut().into_iter().enumerate() {
                        let waiting_this = self.rebinding_pad == Some(i);
                        let badge = if waiting_this { "Press button…" } else { button_name(*button) };
                        if console_binding_row(ui, cw, friendly_control_name(bname), badge, waiting_this) {
                            self.rebinding_pad = Some(i);
                            self.rebinding = None;
                        }
                    }
                });
            }

            _ => {
                // TAB 4: SYSTEM & CHEATS
                ui.label(RichText::new("System & Cheats").size(18.0).strong().color(Color32::from_rgb(238, 238, 242)));
                ui.add_space(10.0);

                let half = (width - 8.0) / 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if console_secondary_button(ui, half, 32.0, "Multiplayer Rooms", true) {
                        self.multiplayer_open = true;
                    }
                    if console_secondary_button(ui, half, 32.0, "Load Camera Image / QR…", true) {
                        actions.push(Action::ChooseCameraPicture);
                    }
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if let Some(pid) = program_id {
                        if console_secondary_button(ui, half, 32.0, "Open Mods Folder", true) {
                            actions.push(Action::Mods(pid));
                        }
                    } else if console_secondary_button(ui, half, 32.0, "Open Cheats Folder", true) {
                        actions.push(Action::CheatsFolder);
                    }
                    if console_secondary_button(ui, half, 32.0, "Back to Launcher", true) {
                        actions.push(Action::Library);
                    }
                });

                ui.add_space(8.0);
                if console_secondary_button(ui, width, 32.0, "Clean Residual Build Files…", true) {
                    actions.push(Action::CleanResiduals);
                }

                ui.add_space(12.0);

                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Action Replay Cheats").size(14.0).strong().color(Color32::from_rgb(236, 236, 240)));
                    ui.add_space(6.0);
                    let btn_half = (cw - 8.0) / 2.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if console_secondary_button(ui, btn_half, 30.0, "Reload Cheats", true) {
                            actions.push(Action::ReloadCheats);
                        }
                        if console_secondary_button(ui, btn_half, 30.0, "Open Cheats Folder", true) {
                            actions.push(Action::CheatsFolder);
                        }
                    });
                    ui.add_space(8.0);

                    if let Some(cheat_list) = cheats {
                        let user_cheats: Vec<_> = cheat_list.iter().enumerate().filter(|(_, c)| !c.3).collect();
                        if user_cheats.is_empty() {
                            ui.label(
                                RichText::new("No custom cheat codes configured.")
                                    .size(12.5)
                                    .color(Color32::from_rgb(138, 138, 148)),
                            );
                        } else {
                            for (idx, (cname, enabled, notes, _)) in user_cheats {
                                let mut on = *enabled;
                                if console_toggle_row(ui, cw, cname, notes, &mut on) {
                                    actions.push(Action::Cheat(idx, on));
                                }
                                ui.add_space(4.0);
                            }
                        }
                    }

                    ui.add_space(8.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.cheat_name)
                            .hint_text("Cheat name")
                            .margin(egui::Margin::symmetric(8, 6))
                            .desired_width(cw),
                    );
                    ui.add_space(6.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.cheat_code)
                            .hint_text("00000000 00000000")
                            .margin(egui::Margin::symmetric(8, 6))
                            .desired_rows(3)
                            .desired_width(cw)
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.add_space(6.0);
                    if console_primary_button(ui, 132.0, 30.0, "Add Cheat") && !self.cheat_code.trim().is_empty() {
                        actions.push(Action::AddCheat(
                            std::mem::take(&mut self.cheat_name),
                            std::mem::take(&mut self.cheat_code),
                        ));
                    }
                });
            }
        }
    }"#;

const WIDE_CONSOLE_WIDGET_HELPERS_RS: &str = r#"
fn friendly_control_name(raw: &str) -> &str {
    raw
}

fn can_install_to_applications() -> bool {
    #[cfg(target_os = "macos")]
    {
        if std::env::var_os("CI").is_some() {
            return false;
        }
        let Ok(exe) = std::env::current_exe() else { return false };
        let Some(bundle) = exe.ancestors().nth(3) else { return false };
        if bundle.extension().and_then(|e| e.to_str()) != Some("app") {
            return false;
        }
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        !bundle.starts_with("/Applications") && !bundle.starts_with(home.join("Applications"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Generates an original, 100% royalty-free scenic Hoenn-inspired Sapphire Ocean & Underwater Shrine background image.
fn blueorb_generate_ocean_scenery(w: usize, h: usize) -> egui::ColorImage {
    let mut pixels = vec![0u8; w * h * 4];
    let wf = w as f32;
    let hf = h as f32;

    for y in 0..h {
        let ny = (y as f32 + 0.5) / hf; // 0 = top, 1 = bottom
        for x in 0..w {
            let nx = (x as f32 + 0.5) / wf;
            let idx = (y * w + x) * 4;

            // Gentle ocean surface line across y ~ 0.24
            let horizon = 0.245 + 0.005 * (nx * 14.0).sin() + 0.003 * (nx * 29.0).cos();

            let (mut r, mut g, mut b) = if ny < horizon {
                // Twilight sapphire sky gradient above ocean horizon
                let t = (ny / horizon).clamp(0.0, 1.0);
                let sky_r = 12.0 + 36.0 * t * t;
                let sky_g = 24.0 + 78.0 * t * t;
                let sky_b = 58.0 + 114.0 * t;

                // Subtle twilight stars near the upper sky
                let sx = (nx * 180.0).floor();
                let sy = (ny * 120.0).floor();
                let hash = ((sx * 127.1 + sy * 311.7).sin() * 43758.547).fract().abs();
                let star = if ny < horizon * 0.72 && hash > 0.9965 {
                    (1.0 - ny / (horizon * 0.75)) * 120.0
                } else {
                    0.0
                };

                // Horizon cyan mist glow around x ~ 0.52
                let sun_dx = (nx - 0.52) / 0.36;
                let glow = (1.0 - sun_dx * sun_dx).clamp(0.0, 1.0) * t * t * t;

                // Distant volcanic island silhouettes at horizon
                let island_l = (-((nx - 0.14) / 0.09).powi(2)).exp() * 0.075
                    + (-((nx - 0.23) / 0.05).powi(2)).exp() * 0.04;
                let island_r = (-((nx - 0.85) / 0.10).powi(2)).exp() * 0.085
                    + (-((nx - 0.74) / 0.05).powi(2)).exp() * 0.038;
                let ridge_y = horizon - (island_l + island_r).max(0.0);
                if ny > ridge_y {
                    (11.0, 22.0, 44.0)
                } else {
                    (
                        sky_r + glow * 45.0 + star,
                        sky_g + glow * 105.0 + star,
                        sky_b + glow * 135.0 + star,
                    )
                }
            } else {
                // Deep cobalt-to-abyssal sapphire ocean water depth
                let depth = ((ny - horizon) / (1.0 - horizon)).clamp(0.0, 1.0);
                let mut wr = 18.0 * (1.0 - depth) + 5.0 * depth;
                let mut wg = 86.0 * (1.0 - depth).powf(1.2) + 18.0 * depth;
                let mut wb = 166.0 * (1.0 - depth).powf(0.85) + 42.0 * depth;

                // Crisp bright surface foam/shimmer along the waterline
                let surf = (-(ny - horizon) * 65.0).exp();
                let shimmer = 0.5 + 0.5 * (nx * 46.0 + (ny * 90.0).sin()).sin();
                wr += surf * (42.0 + 38.0 * shimmer);
                wg += surf * (110.0 + 55.0 * shimmer);
                wb += surf * (135.0 + 35.0 * shimmer);

                // Volumetric diagonal sunbeams piercing down into the water
                let proj = (nx - 0.52) - (ny - horizon) * 0.34;
                let beams = (0.5 + 0.5 * (proj * 26.0 + (proj * 9.0).sin() * 0.6).cos()).powf(3.0);
                let beam_env = (-((proj) / 0.42).powi(2)).exp() * (1.0 - depth * 0.82);
                let ray = beams * beam_env;
                wr += ray * 24.0;
                wg += ray * 68.0;
                wb += ray * 98.0;

                // Subtle underwater caustics in upper half of water column
                let c1 = ((nx * 24.0 + ny * 18.0).sin() * (nx * 17.0 - ny * 26.0).cos()).abs();
                let caustic = c1.powf(4.0) * (1.0 - depth).powi(2) * 0.42;
                wg += caustic * 54.0;
                wb += caustic * 78.0;

                // Silhouetted ancient submerged stone pillars on far left & right edges (keeps center open)
                let pillar_l = nx > 0.04 && nx < 0.105 && ny > 0.44;
                let pillar_r = nx > 0.895 && nx < 0.96 && ny > 0.40;
                let step_l = nx < 0.145 && ny > 0.84;
                let step_r = nx > 0.855 && ny > 0.82;
                if pillar_l || pillar_r || step_l || step_r {
                    let stone_grad = 0.7 + 0.3 * (1.0 - depth);
                    wr = 7.0 * stone_grad;
                    wg = 18.0 * stone_grad;
                    wb = 38.0 * stone_grad;

                    // Faint glowing cyan geometric rune bands on the pillars
                    let on_l_band = pillar_l && ((ny - 0.56).abs() < 0.008 || (ny - 0.68).abs() < 0.008);
                    let on_r_band = pillar_r && ((ny - 0.52).abs() < 0.008 || (ny - 0.64).abs() < 0.008);
                    if (on_l_band && (nx - 0.0725).abs() < 0.018) || (on_r_band && (nx - 0.9275).abs() < 0.018) {
                        wr = 52.0;
                        wg = 195.0;
                        wb = 248.0;
                    }
                }

                // Sandy seabed floor along bottom edge
                let floor_y = 0.91 + 0.018 * (nx * 9.0).sin() + 0.008 * (nx * 23.0).cos();
                if ny > floor_y {
                    let f_blend = ((ny - floor_y) / 0.04).clamp(0.0, 1.0);
                    let sand = 0.82 + 0.18 * (nx * 35.0 + ny * 40.0).sin();
                    wr = wr * (1.0 - f_blend) + (12.0 * sand) * f_blend;
                    wg = wg * (1.0 - f_blend) + (30.0 * sand) * f_blend;
                    wb = wb * (1.0 - f_blend) + (58.0 * sand) * f_blend;
                }

                (wr, wg, wb)
            };

            // Soft vignette so centered launcher UI panel pops with crisp readability
            let vx = (nx - 0.5) * 2.0;
            let vy = (ny - 0.5) * 2.0;
            let vig = (1.0 - (vx * vx + vy * vy) * 0.24).clamp(0.55, 1.0);
            r *= vig;
            g *= vig;
            b *= vig;

            pixels[idx] = r.round().clamp(0.0, 255.0) as u8;
            pixels[idx + 1] = g.round().clamp(0.0, 255.0) as u8;
            pixels[idx + 2] = b.round().clamp(0.0, 255.0) as u8;
            pixels[idx + 3] = 255;
        }
    }

    egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels)
}

fn paint_diveball_emblem(painter: &egui::Painter, center: egui::Pos2, r: f32) {
    // Clean geometric Dive-Ball vector icon painted directly via egui primitives
    painter.circle_filled(center, r, Color32::from_rgb(232, 238, 246));
    // Upper Sapphire hemisphere clip
    let top_clip = egui::Rect::from_min_max(
        egui::pos2(center.x - r - 1.0, center.y - r - 1.0),
        egui::pos2(center.x + r + 1.0, center.y - r * 0.04),
    );
    let tp = painter.with_clip_rect(top_clip);
    tp.circle_filled(center, r, Color32::from_rgb(24, 108, 218));
    // Aqua upper cap
    let cap_clip = egui::Rect::from_min_max(
        egui::pos2(center.x - r - 1.0, center.y - r - 1.0),
        egui::pos2(center.x + r + 1.0, center.y - r * 0.48),
    );
    painter.with_clip_rect(cap_clip).circle_filled(center, r, Color32::from_rgb(86, 184, 248));
    // White wave accent band
    let wave_clip = egui::Rect::from_min_max(
        egui::pos2(center.x - r - 1.0, center.y - r * 0.25),
        egui::pos2(center.x + r + 1.0, center.y - r * 0.14),
    );
    painter.with_clip_rect(wave_clip).circle_filled(center, r * 0.96, Color32::from_rgb(212, 242, 255));

    // Equatorial dark seam
    let seam_h = (r * 0.16).max(1.8);
    let ball_clip = egui::Rect::from_center_size(center, Vec2::splat(r * 2.0));
    painter.with_clip_rect(ball_clip).rect_filled(
        egui::Rect::from_center_size(center, Vec2::new(r * 2.0, seam_h)),
        egui::CornerRadius::ZERO,
        Color32::from_rgb(24, 28, 36),
    );
    painter.circle_stroke(center, r, egui::Stroke::new(1.0, Color32::from_rgb(20, 24, 32)));

    // Center clasp ring + sapphire button
    painter.circle_filled(center, r * 0.36, Color32::from_rgb(24, 28, 36));
    painter.circle_filled(center, r * 0.25, Color32::from_rgb(240, 244, 250));
    painter.circle_filled(center, r * 0.14, Color32::from_rgb(56, 182, 255));
}

fn console_nav_button(ui: &mut egui::Ui, width: f32, label: &str, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 32.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered();
        let bg = if active {
            Color32::from_rgb(36, 36, 42)
        } else if hovered {
            Color32::from_rgb(28, 28, 33)
        } else {
            Color32::TRANSPARENT
        };
        if bg != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(4), bg);
        }
        if active {
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 2.0, rect.top() + 7.0),
                Vec2::new(2.5, rect.height() - 14.0),
            );
            ui.painter().rect_filled(bar, egui::CornerRadius::same(1), Color32::from_rgb(82, 138, 212));
        }
        let text_col = if active || hovered {
            Color32::from_rgb(242, 242, 246)
        } else {
            Color32::from_rgb(164, 164, 174)
        };
        ui.painter().text(
            egui::pos2(rect.left() + 12.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            text_col,
        );
    }
    resp.clicked()
}

fn console_primary_button(ui: &mut egui::Ui, width: f32, height: f32, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if resp.is_pointer_button_down_on() {
            Color32::from_rgb(46, 82, 132)
        } else if resp.hovered() {
            Color32::from_rgb(66, 112, 174)
        } else {
            Color32::from_rgb(56, 98, 154)
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(4), fill);
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.0, Color32::from_rgb(86, 134, 198)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(13.0),
            Color32::WHITE,
        );
    }
    resp.clicked()
}

fn console_secondary_button(ui: &mut egui::Ui, width: f32, height: f32, label: &str, enabled: bool) -> bool {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), sense);
    if ui.is_rect_visible(rect) {
        let fill = if !enabled {
            Color32::from_rgb(28, 28, 32)
        } else if resp.is_pointer_button_down_on() {
            Color32::from_rgb(54, 54, 62)
        } else if resp.hovered() {
            Color32::from_rgb(46, 46, 53)
        } else {
            Color32::from_rgb(38, 38, 44)
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(4), fill);
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.0, Color32::from_rgb(62, 62, 71)),
            egui::StrokeKind::Inside,
        );
        let col = if enabled {
            Color32::from_rgb(232, 232, 238)
        } else {
            Color32::from_rgb(104, 104, 114)
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.5),
            col,
        );
    }
    enabled && resp.clicked()
}

fn console_danger_button(ui: &mut egui::Ui, width: f32, height: f32, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if resp.hovered() {
            Color32::from_rgb(72, 32, 34)
        } else {
            Color32::from_rgb(46, 25, 27)
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(4), fill);
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.0, Color32::from_rgb(104, 50, 54)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.5),
            Color32::from_rgb(244, 168, 168),
        );
    }
    resp.clicked()
}

fn console_card(ui: &mut egui::Ui, width: f32, body: impl FnOnce(&mut egui::Ui, f32)) {
    egui::Frame::NONE
        .fill(Color32::from_rgb(30, 30, 34))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(50, 50, 57)))
        .corner_radius(egui::CornerRadius::same(5))
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            let cw = (width - 30.0).max(160.0);
            ui.set_min_width(cw);
            ui.set_max_width(cw);
            body(ui, cw);
        });
}

fn console_segmented<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    current: T,
    options: &[(T, &str)],
    id_salt: &str,
) -> Option<T> {
    let mut picked = None;
    let (outer_rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    if ui.is_rect_visible(outer_rect) {
        ui.painter().rect_filled(outer_rect, egui::CornerRadius::same(4), Color32::from_rgb(20, 20, 23));
        ui.painter().rect_stroke(
            outer_rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.0, Color32::from_rgb(48, 48, 55)),
            egui::StrokeKind::Inside,
        );
    }
    let inner = outer_rect.shrink(3.0);
    let n = options.len().max(1) as f32;
    let gap = 3.0;
    let item_w = ((inner.width() - gap * (n - 1.0)) / n).max(12.0);

    for (idx, &(val, label)) in options.iter().enumerate() {
        let x0 = inner.left() + (idx as f32) * (item_w + gap);
        let item_rect = egui::Rect::from_min_size(egui::pos2(x0, inner.top()), Vec2::new(item_w, inner.height()));
        let resp = ui.interact(item_rect, ui.make_persistent_id((id_salt, idx)), egui::Sense::click());
        let selected = val == current;
        if ui.is_rect_visible(item_rect) {
            if selected {
                ui.painter().rect_filled(item_rect, egui::CornerRadius::same(3), Color32::from_rgb(56, 98, 154));
            } else if resp.hovered() {
                ui.painter().rect_filled(item_rect, egui::CornerRadius::same(3), Color32::from_rgb(34, 34, 40));
            }
            let col = if selected || resp.hovered() {
                Color32::WHITE
            } else {
                Color32::from_rgb(168, 168, 178)
            };
            ui.painter().text(
                item_rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(12.5),
                col,
            );
        }
        if resp.clicked() && !selected {
            picked = Some(val);
        }
    }
    picked
}

fn console_toggle_row(ui: &mut egui::Ui, width: f32, title: &str, subtitle: &str, value: &mut bool) -> bool {
    let row_h = if subtitle.is_empty() { 28.0 } else { 40.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, row_h), egui::Sense::click());
    let mut changed = false;
    if resp.clicked() {
        *value = !*value;
        changed = true;
    }
    if ui.is_rect_visible(rect) {
        let switch_w = 40.0;
        let switch_h = 22.0;
        let switch_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - switch_w, rect.center().y - switch_h * 0.5),
            Vec2::new(switch_w, switch_h),
        );
        let track = if *value {
            Color32::from_rgb(56, 98, 154)
        } else if resp.hovered() {
            Color32::from_rgb(56, 56, 64)
        } else {
            Color32::from_rgb(42, 42, 49)
        };
        ui.painter().rect_filled(switch_rect, egui::CornerRadius::same(11), track);
        let knob_x = if *value { switch_rect.right() - 11.0 } else { switch_rect.left() + 11.0 };
        ui.painter().circle_filled(egui::pos2(knob_x, switch_rect.center().y), 8.0, Color32::WHITE);

        let text_clip = egui::Rect::from_min_max(rect.min, egui::pos2(switch_rect.left() - 12.0, rect.bottom()));
        let tp = ui.painter().with_clip_rect(text_clip);
        if subtitle.is_empty() {
            tp.text(
                egui::pos2(rect.left(), rect.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(13.0),
                Color32::from_rgb(232, 232, 238),
            );
        } else {
            tp.text(
                egui::pos2(rect.left(), rect.top() + 10.0),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(13.0),
                Color32::from_rgb(232, 232, 238),
            );
            tp.text(
                egui::pos2(rect.left(), rect.top() + 28.0),
                egui::Align2::LEFT_CENTER,
                subtitle,
                egui::FontId::proportional(11.5),
                Color32::from_rgb(138, 138, 148),
            );
        }
    }
    changed
}

fn console_binding_row(ui: &mut egui::Ui, width: f32, label: &str, badge: &str, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 30.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(4), Color32::from_rgb(36, 36, 42));
        }
        ui.painter().text(
            egui::pos2(rect.left() + 4.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            Color32::from_rgb(224, 224, 232),
        );
        let badge_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 120.0, rect.center().y - 12.0),
            Vec2::new(116.0, 24.0),
        );
        let fill = if active {
            Color32::from_rgb(56, 98, 154)
        } else {
            Color32::from_rgb(20, 20, 23)
        };
        ui.painter().rect_filled(badge_rect, egui::CornerRadius::same(4), fill);
        ui.painter().rect_stroke(
            badge_rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.0, Color32::from_rgb(52, 52, 60)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            badge_rect.center(),
            egui::Align2::CENTER_CENTER,
            badge,
            egui::FontId::monospace(12.0),
            Color32::WHITE,
        );
    }
    resp.clicked()
}

fn console_screen_layout_card(
    ui: &mut egui::Ui,
    size: Vec2,
    title: &str,
    subtitle: &str,
    kind: u8,
    selected: bool,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if selected {
            Color32::from_rgb(30, 40, 56)
        } else if resp.hovered() {
            Color32::from_rgb(36, 36, 42)
        } else {
            Color32::from_rgb(30, 30, 34)
        };
        let border = if selected {
            egui::Stroke::new(1.5, Color32::from_rgb(82, 138, 212))
        } else if resp.hovered() {
            egui::Stroke::new(1.0, Color32::from_rgb(74, 74, 84))
        } else {
            egui::Stroke::new(1.0, Color32::from_rgb(50, 50, 57))
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(5), bg);
        ui.painter().rect_stroke(rect, egui::CornerRadius::same(5), border, egui::StrokeKind::Inside);

        let cx = rect.left() + 32.0;
        let cy = rect.center().y;
        let top_fill = if selected {
            Color32::from_rgb(104, 168, 248)
        } else {
            Color32::from_rgb(142, 142, 154)
        };
        let bot_fill = if selected {
            Color32::from_rgba_unmultiplied(104, 168, 248, 165)
        } else {
            Color32::from_rgba_unmultiplied(142, 142, 154, 140)
        };

        match kind {
            0 => {
                let r_top = egui::Rect::from_center_size(egui::pos2(cx, cy - 9.0), Vec2::new(30.0, 15.0));
                let r_bot = egui::Rect::from_center_size(egui::pos2(cx, cy + 9.0), Vec2::new(23.0, 13.0));
                ui.painter().rect_filled(r_top, egui::CornerRadius::same(2), top_fill);
                ui.painter().rect_filled(r_bot, egui::CornerRadius::same(2), bot_fill);
            }
            1 => {
                let r_top = egui::Rect::from_center_size(egui::pos2(cx - 7.0, cy), Vec2::new(28.0, 19.0));
                let r_bot = egui::Rect::from_center_size(egui::pos2(cx + 14.0, cy + 4.0), Vec2::new(13.0, 11.0));
                ui.painter().rect_filled(r_top, egui::CornerRadius::same(2), top_fill);
                ui.painter().rect_filled(r_bot, egui::CornerRadius::same(2), bot_fill);
            }
            2 => {
                let r_top = egui::Rect::from_center_size(egui::pos2(cx - 11.0, cy), Vec2::new(21.0, 14.0));
                let r_bot = egui::Rect::from_center_size(egui::pos2(cx + 11.0, cy), Vec2::new(18.0, 14.0));
                ui.painter().rect_filled(r_top, egui::CornerRadius::same(2), top_fill);
                ui.painter().rect_filled(r_bot, egui::CornerRadius::same(2), bot_fill);
            }
            _ => {
                let w1 = egui::Rect::from_center_size(egui::pos2(cx - 7.0, cy - 4.0), Vec2::new(27.0, 17.0));
                let w2 = egui::Rect::from_center_size(egui::pos2(cx + 9.0, cy + 5.0), Vec2::new(22.0, 15.0));
                ui.painter().rect_filled(w1, egui::CornerRadius::same(2), top_fill);
                ui.painter().rect_filled(w2.expand(2.0), egui::CornerRadius::same(3), bg);
                ui.painter().rect_filled(w2, egui::CornerRadius::same(2), bot_fill);
            }
        }

        let text_clip = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 64.0, rect.top() + 4.0),
            egui::pos2(rect.right() - 8.0, rect.bottom() - 4.0),
        );
        let tp = ui.painter().with_clip_rect(text_clip);
        tp.text(
            egui::pos2(rect.left() + 64.0, rect.center().y - 8.0),
            egui::Align2::LEFT_CENTER,
            title,
            egui::FontId::proportional(13.0),
            Color32::from_rgb(236, 236, 240),
        );
        tp.text(
            egui::pos2(rect.left() + 64.0, rect.center().y + 10.0),
            egui::Align2::LEFT_CENTER,
            subtitle,
            egui::FontId::proportional(11.5),
            Color32::from_rgb(138, 138, 148),
        );
    }
    resp.clicked()
}
"#;

const MOVE_TO_APPLICATIONS_RS: &str = r#"
// BLUEORB_MOVE_TO_APPLICATIONS
/// On first launch outside /Applications, offers to install the bundle there (like most Mac apps do).
#[cfg(target_os = "macos")]
fn blueorb_offer_move_to_applications() {
    use std::path::{Path, PathBuf};

    if std::env::var_os("CI").is_some() || std::env::args().skip(1).any(|a| a.starts_with('-')) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    // .../BlueOrb.app/Contents/MacOS/BlueOrb
    let Some(bundle) = exe.ancestors().nth(3).map(Path::to_path_buf) else { return };
    if bundle.extension().and_then(|e| e.to_str()) != Some("app") {
        return;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if bundle.starts_with("/Applications") || bundle.starts_with(home.join("Applications")) {
        return;
    }
    let stamp_dir = home.join("Library/Application Support/BlueOrb");
    let stamp = stamp_dir.join(".skip_move_to_applications");
    if stamp.exists() {
        return;
    }

    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title("Move to Applications folder?")
        .set_description(
            "BlueOrb is not in your Applications folder.\n\nMove it there now so it appears in Launchpad and Spotlight?",
        )
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    if answer != rfd::MessageDialogResult::Yes {
        let _ = std::fs::create_dir_all(&stamp_dir);
        let _ = std::fs::write(&stamp, b"1");
        return;
    }

    let name = bundle.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "BlueOrb.app".into());
    let copy_to = |dir: &Path| -> Option<PathBuf> {
        let dest = dir.join(&name);
        if dest.exists() && std::fs::remove_dir_all(&dest).is_err() {
            return None;
        }
        let ok = std::process::Command::new("/usr/bin/ditto")
            .arg(&bundle)
            .arg(&dest)
            .status()
            .is_ok_and(|s| s.success());
        ok.then_some(dest)
    };
    let user_apps = home.join("Applications");
    let dest = copy_to(Path::new("/Applications")).or_else(|| {
        let _ = std::fs::create_dir_all(&user_apps);
        copy_to(&user_apps)
    });
    let Some(dest) = dest else {
        let _ = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Could not move BlueOrb")
            .set_description("BlueOrb could not be copied to your Applications folder. You can drag it there manually.")
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
        return;
    };

    // a quarantined copy launched from a randomized read-only path can't be deleted, leave it alone
    if !bundle.to_string_lossy().contains("/AppTranslocation/") {
        let _ = std::fs::remove_dir_all(&bundle);
    }
    if std::process::Command::new("/usr/bin/open").arg("-n").arg(&dest).spawn().is_ok() {
        std::process::exit(0);
    }
}
"#;

const OFFICIAL_ICON_UPDATER_RS: &str = r#"
#[cfg(target_os = "macos")]
fn blueorb_move_bundle_to_applications_now() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(bundle) = exe.ancestors().nth(3).map(Path::to_path_buf) else { return };
    if bundle.extension().and_then(|e| e.to_str()) != Some("app") {
        return;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if bundle.starts_with("/Applications") || bundle.starts_with(home.join("Applications")) {
        return;
    }
    let name = bundle.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "BlueOrb.app".into());
    let copy_to = |dir: &Path| -> Option<PathBuf> {
        let dest = dir.join(&name);
        if dest.exists() && std::fs::remove_dir_all(&dest).is_err() {
            return None;
        }
        let ok = std::process::Command::new("/usr/bin/ditto")
            .arg(&bundle)
            .arg(&dest)
            .status()
            .is_ok_and(|s| s.success());
        ok.then_some(dest)
    };
    let user_apps = home.join("Applications");
    let dest = copy_to(Path::new("/Applications")).or_else(|| {
        let _ = std::fs::create_dir_all(&user_apps);
        copy_to(&user_apps)
    });
    let Some(dest) = dest else {
        let _ = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Could not move BlueOrb")
            .set_description("BlueOrb could not be copied to your Applications folder. You can drag it there manually.")
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
        return;
    };
    if !bundle.to_string_lossy().contains("/AppTranslocation/") {
        let _ = std::fs::remove_dir_all(&bundle);
    }
    if std::process::Command::new("/usr/bin/open").arg("-n").arg(&dest).spawn().is_ok() {
        std::process::exit(0);
    }
}
/// Returns the official Alpha Sapphire ROM icon if a recompiled ROM is already configured,
/// or the procedural Sapphire Dive-Ball launcher icon prior to recompilation.
fn blueorb_initial_window_icon(rom_path: Option<&Path>) -> Option<winit::window::Icon> {
    if let Some(rom) = rom_path.filter(|p| p.is_file()) {
        if let Ok(title) = zakuro_fs::Title::load(rom) {
            let pid = title.program_id();
            if zakuro_core::recompiled::installed(pid).is_some() {
                if let Some(smdh) = title.exefs_file("icon").and_then(library::Smdh::parse) {
                    if smdh.icon.len() == 48 * 48 * 4 {
                        let rgba = blueorb_upscale_smdh_rgba(&smdh.icon, 64);
                        if let Ok(ic) = winit::window::Icon::from_rgba(rgba, 64, 64) {
                            return Some(ic);
                        }
                    }
                }
            }
        }
    }
    let rgba = blueorb_render_diveball_rgba(64);
    winit::window::Icon::from_rgba(rgba, 64, 64).ok()
}

fn blueorb_render_diveball_rgba(size: usize) -> Vec<u8> {
    let mut dst = vec![0u8; size * size * 4];
    let szf = size as f32;
    for y in 0..size {
        let uy = ((y as f32 + 0.5) / szf) * 2.0 - 1.0;
        for x in 0..size {
            let ux = ((x as f32 + 0.5) / szf) * 2.0 - 1.0;
            let idx = (y * size + x) * 4;
            let r_ball = 0.88f32;
            let d2 = ux * ux + uy * uy;
            if d2 > r_ball * r_ball {
                continue;
            }
            let d = d2.sqrt();
            let aa = ((r_ball - d) * szf * 0.5).clamp(0.0, 1.0);
            let nx = ux / r_ball;
            let ny = -uy / r_ball;
            let nz = (1.0 - (nx * nx + ny * ny).min(1.0)).sqrt();
            let py = ny * 0.98 - nz * (-0.19);
            let pz = ny * (-0.19) + nz * 0.98;
            let clasp_ang = pz.clamp(-1.0, 1.0).acos();
            let shade = (0.38 + 0.62 * (nx * -0.4 + ny * 0.56 + nz * 0.72).max(0.0)).clamp(0.0, 1.0);

            let (r, g, b) = if clasp_ang < 0.11 {
                (56.0, 189.0, 255.0)
            } else if clasp_ang < 0.21 {
                (238.0 * shade, 242.0 * shade, 248.0 * shade)
            } else if clasp_ang < 0.28 || py.abs() < 0.07 {
                (26.0, 30.0, 40.0)
            } else if py > 0.0 {
                let wave = 0.23 + 0.08 * (nx * 5.0).cos();
                if (py - wave).abs() < 0.05 {
                    (205.0, 240.0, 255.0)
                } else if py > 0.55 {
                    (64.0 * shade + 20.0, 168.0 * shade + 30.0, 248.0)
                } else {
                    (18.0 * shade + 8.0, 88.0 * shade + 24.0, 212.0 * shade + 40.0)
                }
            } else {
                (238.0 * shade, 242.0 * shade, 248.0 * shade)
            };
            dst[idx] = r.clamp(0.0, 255.0) as u8;
            dst[idx + 1] = g.clamp(0.0, 255.0) as u8;
            dst[idx + 2] = b.clamp(0.0, 255.0) as u8;
            dst[idx + 3] = (aa * 255.0) as u8;
        }
    }
    dst
}

/// Smooth 2x EPX + bilinear squircle upscaler turning the ROM's official 48x48 SMDH icon into crisp HD RGBA.
fn blueorb_upscale_smdh_rgba(src48: &[u8], out_size: usize) -> Vec<u8> {
    let sim = |a: &[u8], b: &[u8]| -> bool {
        (a[0] as i32 - b[0] as i32).abs() * 3
            + (a[1] as i32 - b[1] as i32).abs() * 4
            + (a[2] as i32 - b[2] as i32).abs() * 2
            < 160
    };
    let px48 = |x: usize, y: usize| -> [u8; 4] {
        let i = (y.min(47) * 48 + x.min(47)) * 4;
        [src48[i], src48[i + 1], src48[i + 2], 255]
    };
    let mut s96 = vec![[0u8; 4]; 96 * 96];
    for y in 0..48 {
        for x in 0..48 {
            let p = px48(x, y);
            let a = px48(x, y.saturating_sub(1));
            let b = px48(x + 1, y);
            let c = px48(x.saturating_sub(1), y);
            let d = px48(x, y + 1);
            let blend = |q: [u8; 4], r: [u8; 4]| [
                ((q[0] as u16 + r[0] as u16 + p[0] as u16 * 2) / 4) as u8,
                ((q[1] as u16 + r[1] as u16 + p[1] as u16 * 2) / 4) as u8,
                ((q[2] as u16 + r[2] as u16 + p[2] as u16 * 2) / 4) as u8,
                255,
            ];
            s96[(y * 2) * 96 + x * 2] = if sim(&c, &a) && !sim(&c, &d) && !sim(&a, &b) { blend(c, a) } else { p };
            s96[(y * 2) * 96 + x * 2 + 1] = if sim(&a, &b) && !sim(&c, &a) && !sim(&b, &d) { blend(a, b) } else { p };
            s96[(y * 2 + 1) * 96 + x * 2] = if sim(&d, &c) && !sim(&b, &d) && !sim(&c, &a) { blend(d, c) } else { p };
            s96[(y * 2 + 1) * 96 + x * 2 + 1] = if sim(&b, &d) && !sim(&a, &b) && !sim(&d, &c) { blend(b, d) } else { p };
        }
    }

    let mut dst = vec![0u8; out_size * out_size * 4];
    let szf = out_size as f32;
    let rad = szf * 0.22;
    for y in 0..out_size {
        let yf = y as f32 + 0.5;
        let gy = ((yf / szf) * 96.0 - 0.5).clamp(0.0, 95.0);
        let y0 = gy.floor() as usize;
        let y1 = (y0 + 1).min(95);
        let ty = gy - y0 as f32;
        for x in 0..out_size {
            let xf = x as f32 + 0.5;
            let gx = ((xf / szf) * 96.0 - 0.5).clamp(0.0, 95.0);
            let x0 = gx.floor() as usize;
            let x1 = (x0 + 1).min(95);
            let tx = gx - x0 as f32;

            let c00 = s96[y0 * 96 + x0];
            let c10 = s96[y0 * 96 + x1];
            let c01 = s96[y1 * 96 + x0];
            let c11 = s96[y1 * 96 + x1];

            let dx = if xf < rad { rad - xf } else if xf > szf - rad { xf - (szf - rad) } else { 0.0 };
            let dy = if yf < rad { rad - yf } else if yf > szf - rad { yf - (szf - rad) } else { 0.0 };
            let cd = (dx * dx + dy * dy).sqrt();
            let alpha = ((rad + 0.5 - cd) * 255.0).round().clamp(0.0, 255.0) as u8;

            let o = (y * out_size + x) * 4;
            for ch in 0..3 {
                let top = c00[ch] as f32 * (1.0 - tx) + c10[ch] as f32 * tx;
                let bot = c01[ch] as f32 * (1.0 - tx) + c11[ch] as f32 * tx;
                dst[o + ch] = (top * (1.0 - ty) + bot * ty).round().clamp(0.0, 255.0) as u8;
            }
            dst[o + 3] = alpha;
        }
    }
    dst
}

fn blueorb_encode_png(size: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(rgba, size, size, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(out)
}

/// Replaces the bundle's placeholder icon on disk with the user's official ROM SMDH icon once recompiled.
fn blueorb_persist_official_bundle_icon(icon48: &[u8], game_title: &str) {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(exe_dir) = exe.parent() else { return };

    let rgba256 = blueorb_upscale_smdh_rgba(icon48, 256);
    let Some(png256) = blueorb_encode_png(256, &rgba256) else { return };

    #[cfg(target_os = "macos")]
    {
        if exe_dir.ends_with("Contents/MacOS") {
            if let Some(contents) = exe_dir.parent() {
                let res_dir = contents.join("Resources");
                let stamp = res_dir.join(".official_icon_applied");
                let icns_path = res_dir.join("AppIcon.icns");
                if !stamp.is_file() && res_dir.is_dir() {
                    let rgba512 = blueorb_upscale_smdh_rgba(icon48, 512);
                    if let Some(png512) = blueorb_encode_png(512, &rgba512) {
                        let total = 8 + (8 + png256.len()) + (8 + png512.len());
                        let mut icns = Vec::with_capacity(total);
                        icns.extend_from_slice(b"icns");
                        icns.extend_from_slice(&(total as u32).to_be_bytes());
                        icns.extend_from_slice(b"ic08");
                        icns.extend_from_slice(&((8 + png256.len()) as u32).to_be_bytes());
                        icns.extend_from_slice(&png256);
                        icns.extend_from_slice(b"ic09");
                        icns.extend_from_slice(&((8 + png512.len()) as u32).to_be_bytes());
                        icns.extend_from_slice(&png512);
                        if std::fs::write(&icns_path, icns).is_ok() {
                            let _ = std::fs::write(&stamp, game_title.as_bytes());
                            if let Some(bundle) = contents.parent() {
                                let _ = std::process::Command::new("codesign")
                                    .args(["--force", "--deep", "--sign", "-"])
                                    .arg(bundle)
                                    .status();
                                let _ = std::process::Command::new("touch").arg(bundle).status();
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let ico_path = exe_dir.join("AppIcon.ico");
        let stamp = exe_dir.join(".official_icon_applied");
        if !stamp.is_file() {
            let mut ico = Vec::with_capacity(22 + png256.len());
            ico.extend_from_slice(&[0, 0, 1, 0, 1, 0, 0, 0, 0, 0]);
            ico.extend_from_slice(&1u16.to_le_bytes());
            ico.extend_from_slice(&32u16.to_le_bytes());
            ico.extend_from_slice(&(png256.len() as u32).to_le_bytes());
            ico.extend_from_slice(&22u32.to_le_bytes());
            ico.extend_from_slice(&png256);
            if std::fs::write(&ico_path, ico).is_ok() {
                let _ = std::fs::write(&stamp, game_title.as_bytes());
                let safe_name: String = game_title
                    .chars()
                    .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
                    .collect();
                let lnk_path = exe_dir.join(format!("{}.lnk", if safe_name.is_empty() { "Pokemon Alpha Sapphire" } else { &safe_name }));
                let ps_script = format!(
                    "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.WorkingDirectory='{}';$s.IconLocation='{}';$s.Save()",
                    lnk_path.display().to_string().replace('\'', "''"),
                    exe.display().to_string().replace('\'', "''"),
                    exe_dir.display().to_string().replace('\'', "''"),
                    ico_path.display().to_string().replace('\'', "''"),
                );
                let _ = std::process::Command::new("powershell")
                    .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &ps_script])
                    .status();
            }
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let png_path = exe_dir.join("AppIcon.png");
        let stamp = exe_dir.join(".official_icon_applied");
        if !stamp.is_file() && std::fs::write(&png_path, &png256).is_ok() {
            let _ = std::fs::write(&stamp, game_title.as_bytes());
            let desktop = format!(
                "[Desktop Entry]\nType=Application\nName={}\nExec=\"{}\"\nIcon={}\nCategories=Game;\nTerminal=false\n",
                game_title,
                exe.display(),
                png_path.display()
            );
            let _ = std::fs::write(exe_dir.join("BlueOrb.desktop"), desktop);
        }
    }
}
"#;

const UPSCALE_HELPERS_RS: &str = r#"
#[inline]
fn blueorb_color_similar(a: [u8; 4], b: [u8; 4]) -> bool {
    let da = (a[3] as i32 - b[3] as i32).abs();
    if a[3] < 16 && b[3] < 16 {
        return true;
    }
    if da > 48 {
        return false;
    }
    let dr = (a[0] as i32 - b[0] as i32).abs();
    let dg = (a[1] as i32 - b[1] as i32).abs();
    let db = (a[2] as i32 - b[2] as i32).abs();
    (dr * 3 + dg * 4 + db * 2) <= 360
}

#[inline]
fn blueorb_mix_rgba(p: [u8; 4], q: [u8; 4]) -> [u8; 4] {
    [
        ((p[0] as u16 * 3 + q[0] as u16 + 2) >> 2) as u8,
        ((p[1] as u16 * 3 + q[1] as u16 + 2) >> 2) as u8,
        ((p[2] as u16 * 3 + q[2] as u16 + 2) >> 2) as u8,
        ((p[3] as u16 * 3 + q[3] as u16 + 2) >> 2) as u8,
    ]
}

#[inline]
fn blueorb_avg_rgba(a: [u8; 4], b: [u8; 4]) -> [u8; 4] {
    [
        ((a[0] as u16 + b[0] as u16 + 1) >> 1) as u8,
        ((a[1] as u16 + b[1] as u16 + 1) >> 1) as u8,
        ((a[2] as u16 + b[2] as u16 + 1) >> 1) as u8,
        ((a[3] as u16 + b[3] as u16 + 1) >> 1) as u8,
    ]
}

fn blueorb_epx_smooth_2x(w: usize, h: usize, src: &[[u8; 4]]) -> Vec<[u8; 4]> {
    let dw = w * 2;
    let dh = h * 2;
    let mut dst = vec![[0u8; 4]; dw * dh];

    for y in 0..h {
        let ym1 = y.saturating_sub(1);
        let yp1 = (y + 1).min(h - 1);
        for x in 0..w {
            let xm1 = x.saturating_sub(1);
            let xp1 = (x + 1).min(w - 1);

            let p = src[y * w + x];
            let a = src[ym1 * w + x];
            let b = src[y * w + xp1];
            let c = src[y * w + xm1];
            let d = src[yp1 * w + x];

            let ca = blueorb_color_similar(c, a);
            let ab = blueorb_color_similar(a, b);
            let bd = blueorb_color_similar(b, d);
            let dc = blueorb_color_similar(d, c);
            let cd = blueorb_color_similar(c, d);

            let e0 = if ca && !cd && !ab { blueorb_mix_rgba(blueorb_avg_rgba(c, a), p) } else { p };
            let e1 = if ab && !ca && !bd { blueorb_mix_rgba(blueorb_avg_rgba(a, b), p) } else { p };
            let e2 = if dc && !bd && !ca { blueorb_mix_rgba(blueorb_avg_rgba(d, c), p) } else { p };
            let e3 = if bd && !ab && !dc { blueorb_mix_rgba(blueorb_avg_rgba(b, d), p) } else { p };

            let out_row0 = (y * 2) * dw + (x * 2);
            let out_row1 = (y * 2 + 1) * dw + (x * 2);
            dst[out_row0] = e0;
            dst[out_row0 + 1] = e1;
            dst[out_row1] = e2;
            dst[out_row1 + 1] = e3;
        }
    }
    dst
}

fn blueorb_upscale_4x_mipmapped(width: u32, height: u32, base_texels: &[[u8; 4]]) -> crate::pack::Picture {
    let w1 = width as usize;
    let h1 = height as usize;
    let mip1 = blueorb_epx_smooth_2x(w1, h1, base_texels);
    let mip0 = blueorb_epx_smooth_2x(w1 * 2, h1 * 2, &mip1);

    let len0 = mip0.len() * 4;
    let len1 = mip1.len() * 4;
    let len2 = base_texels.len() * 4;

    let mut raw = Vec::with_capacity(len0 + len1 + len2);
    for px in &mip0 {
        raw.extend_from_slice(px);
    }
    for px in &mip1 {
        raw.extend_from_slice(px);
    }
    for px in base_texels {
        raw.extend_from_slice(px);
    }

    crate::pack::Picture {
        width: width * 4,
        height: height * 4,
        texels: raw,
        levels: vec![
            crate::pack::Level { offset: 0, width: width * 4, height: height * 4 },
            crate::pack::Level { offset: len0, width: width * 2, height: height * 2 },
            crate::pack::Level { offset: len0 + len1, width, height },
        ],
    }
}
"#;
