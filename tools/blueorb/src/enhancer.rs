// Patchs d'améliorations Remaster HD, Ultra-Wide (16:9 / 21:9 / 32:9), Double Fenêtre détachable (Haut + Bas)
// et Interface Console Large (sans fioritures) appliqués automatiquement aux sources `build/recomp_sdk/zakuro`
// et `build/recomp_sdk/3dsrecomp` sur Linux (x86_64), macOS (ARM64 / Intel) et Windows (x86_64).

use std::fs;
use std::path::Path;

pub fn apply_sdk_enhancements(recomp_repo: &Path, zakuro_repo: &Path) -> Result<(), String> {
    patch_recomp_incremental(recomp_repo)?;
    patch_zakuro_oras_enhancements(zakuro_repo)?;
    patch_zakuro_gpu_ultrawide_upscaler_and_dual_vulkan(zakuro_repo)?;
    patch_zakuro_frontend_console_ui_and_dual_window(zakuro_repo)?;
    Ok(())
}

/// 1. Rend la compilation C de `3dsrecomp` incrémentale : si `.o` est plus récent que `.c` et non vide, on le réutilise.
fn patch_recomp_incremental(recomp_repo: &Path) -> Result<(), String> {
    let compile_rs = recomp_repo.join("src").join("compile.rs");
    if !compile_rs.exists() {
        return Ok(());
    }
    let src = fs::read_to_string(&compile_rs).map_err(|e| e.to_string())?;
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

/// 2. Ajoute l'amélioration vérifiée "No Outlines (HD)" pour Pokémon Saphir Alpha USA Rev 2 (`rev3`).
fn patch_zakuro_oras_enhancements(zakuro_repo: &Path) -> Result<(), String> {
    let enh_path = zakuro_repo
        .join("crates")
        .join("zakuro-core")
        .join("src")
        .join("enhancements.txt");
    if !enh_path.exists() {
        return Ok(());
    }
    let src = fs::read_to_string(&enh_path).map_err(|e| e.to_string())?;
    if src.contains("No Outlines (HD)") {
        return Ok(());
    }

    let target = "= 000400000011C500 Pokémon Alpha Sapphire (USA)\n60 FPS | rev3 | Reshiban | The whole game runs twice as fast with it.\n28C650C1 00000000";
    let updated = "= 000400000011C500 Pokémon Alpha Sapphire (USA)\n\
No Outlines (HD) | rev3 | BlueOrb | Removes black cel-shaded outlines around characters and Pokémon for crisp 4K rendering.\n\
D3000000 00000000\n\
00379EB4 00000000\n\
00379EB8 00000000\n\
D2000000 00000000\n\
60 FPS | rev3 | Reshiban | Turbo 2x engine speed (smooth 60 FPS at 1x speed is already active via Frame Interpolation).\n\
28C650C1 00000000";

    if src.contains(target) {
        fs::write(&enh_path, src.replace(target, updated)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 3. Patchs `zakuro-gpu` :
///    - Échelle de projection 3D Hor+ Ultra-Wide (`16:9`, `21:9`, `32:9`) sans étirer les modèles ni l'UI
///    - Upscaler de textures 4x Smooth-xBR/EPX + Mipmaps trilinéaires + Anisotropie 16x
///    - Constructeur `VulkanPresenter::new_secondary` partageant le même `SharedDevice` Vulkan/Metal (zéro copie) pour la 2e fenêtre !
fn patch_zakuro_gpu_ultrawide_upscaler_and_dual_vulkan(zakuro_repo: &Path) -> Result<(), String> {
    let gpu_lib_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("lib.rs");
    if gpu_lib_rs.exists() {
        let src = fs::read_to_string(&gpu_lib_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_TOP_ASPECT_SCALE") {
            let addition = r#"
// BLUEORB_TOP_ASPECT_SCALE: facteur Hor+ (400.0 / largeur_cible) pour vrai rendu 3D 16:9 / 21:9 / 32:9 sans étirement
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

    let raster_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("raster.rs");
    if raster_rs.exists() {
        let src = fs::read_to_string(&raster_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_ULTRAWIDE_PLACE") {
            let needle = "fn place(registers: &[u32], shaded: &[Vertex], order: Option<&[usize]>, viewport: (f32, f32, f32, f32)) -> Placement {";
            let replacement = r#"fn place(registers: &[u32], shaded_in: &[Vertex], order: Option<&[usize]>, viewport: (f32, f32, f32, f32)) -> Placement {
    // BLUEORB_ULTRAWIDE_PLACE: élargit le champ de vision 3D (Hor+) sur l'écran supérieur (240x400) sans étirer les modèles ni l'interface 2D
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

    let glsl_rs = zakuro_repo
        .join("crates")
        .join("zakuro-gpu")
        .join("src")
        .join("shader")
        .join("glsl.rs");
    if glsl_rs.exists() {
        let src = fs::read_to_string(&glsl_rs).map_err(|e| e.to_string())?;
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

    let hw_rs = zakuro_repo
        .join("crates")
        .join("zakuro-gpu")
        .join("src")
        .join("raster")
        .join("hardware.rs");
    if hw_rs.exists() {
        let mut src = fs::read_to_string(&hw_rs).map_err(|e| e.to_string())?;
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
            let hd_fn_and_orig = r#"    // BLUEORB_HD_UPSCALE: Upscaling temps-réel 4x Smooth-xBR/EPX + Mipmaps trilinéaires pour les textures 3DS
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

    let vk_rs = zakuro_repo
        .join("crates")
        .join("zakuro-gpu")
        .join("src")
        .join("backend")
        .join("vulkan.rs");
    if vk_rs.exists() {
        let mut src = fs::read_to_string(&vk_rs).map_err(|e| e.to_string())?;
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

/// 4. Interface Console Large (façon SteamOS / Zelda64Recomp : 2 colonnes spacieuses, zéro émoji, schémas d'écrans vectoriels)
fn patch_zakuro_frontend_console_ui_and_dual_window(zakuro_repo: &Path) -> Result<(), String> {
    let zakuro_src = zakuro_repo.join("crates").join("zakuro").join("src");

    // 4a. `gui.rs` : Thème sombre neutre sobre & lisible
    let gui_rs = zakuro_src.join("gui.rs");
    if gui_rs.exists() {
        let mut src = fs::read_to_string(&gui_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_CONSOLE_THEME") {
            src = src.replace(
                "        ctx.set_fonts(fonts(system_fonts()));",
                "        ctx.set_fonts(fonts(system_fonts()));\n        apply_blueorb_console_theme(&ctx);",
            );
            src.push_str(CONSOLE_THEME_FN_RS);
            fs::write(&gui_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // 4b. `settings.rs`
    let settings_rs = zakuro_src.join("settings.rs");
    if settings_rs.exists() {
        let mut src = fs::read_to_string(&settings_rs).map_err(|e| e.to_string())?;
        if !src.contains("ZAKURO_SETTINGS_PATH") {
            src = src.replace(
                "fn path() -> Option<PathBuf> {",
                "fn path() -> Option<PathBuf> {\n    if let Some(custom) = std::env::var_os(\"ZAKURO_SETTINGS_PATH\").filter(|v| !v.is_empty()) {\n        return Some(PathBuf::from(custom));\n    }",
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
                "Screens::LargeTop => \"Big top screen, small bottom one\",\n            Screens::SeparateWindows => \"Deux fenêtres séparées\",",
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
                "TopShape::Wide21x9 => \"21:9 Ultra-Wide\",\n            TopShape::Wide32x9 => \"32:9 Super Large\",",
            );
            src = src.replace(
                "TopShape::Wide21x9 => 560.0,",
                "TopShape::Wide21x9 => 560.0,\n            TopShape::Wide32x9 => 240.0 * 32.0 / 9.0,",
            );
        }
        fs::write(&settings_rs, src).map_err(|e| e.to_string())?;
    }

    // 4c. `menus.rs` : Grand Menu Console unifié à onglets latéraux (Partie / Écrans / Graphismes / Contrôles / Avancé)
    let menus_rs = zakuro_src.join("menus.rs");
    if menus_rs.exists() {
        let mut src = fs::read_to_string(&menus_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_WIDE_CONSOLE_MENU") {
            src = src.replace(
                "    CheatsFolder,\n}",
                "    CheatsFolder,\n    ToggleDualWindows,\n    SetLayout(Screens),\n    SetTopShape(TopShape),\n}",
            );
            src = src.replace(
                "    pub menu_open: bool,",
                "    pub menu_open: bool,\n    pub tab: u8,",
            );

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
                return Err("Impossible de localiser `Menus::game` dans `crates/zakuro/src/menus.rs`".into());
            }
            src = src.replace(old_game_fn, WIDE_CONSOLE_GAME_MENU_RS);
            src.push_str(WIDE_CONSOLE_WIDGET_HELPERS_RS);
            fs::write(&menus_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // 4d. `present.rs`
    let present_rs = zakuro_src.join("present.rs");
    if present_rs.exists() {
        let mut src = fs::read_to_string(&present_rs).map_err(|e| e.to_string())?;
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

    // 4e. `lib.rs`
    let lib_rs = zakuro_src.join("lib.rs");
    if lib_rs.exists() {
        let mut src = fs::read_to_string(&lib_rs).map_err(|e| e.to_string())?;
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

            let set_layout_needle = "    fn set_layout(&mut self, layout: Screens) {";
            let set_layout_rep = r#"    fn toggle_dual_windows(&mut self) {
        let target = if self.active.layout == Screens::SeparateWindows {
            self.last_combined_layout
        } else {
            self.last_combined_layout = self.active.layout;
            Screens::SeparateWindows
        };
        self.set_layout(target);
        self.menus.toast = Some((
            if target == Screens::SeparateWindows {
                "Écrans séparés sur deux fenêtres".to_owned()
            } else {
                "Écrans regroupés dans une fenêtre".to_owned()
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
        self.menus.toast = Some((format!("Format : {}", shape.name()), Instant::now()));
    }

    fn set_layout(&mut self, layout: Screens) {
        if layout != Screens::SeparateWindows {
            self.last_combined_layout = layout;
        }"#;
            src = src.replace(set_layout_needle, set_layout_rep);

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
                "            Action::ToggleDualWindows => self.toggle_dual_windows(),\n            Action::SetLayout(layout) => self.set_layout(layout),\n            Action::SetTopShape(shape) => self.set_top_shape(shape),\n            Action::Quit => event_loop.exit(),",
            );

            src = src.replace(
                "Some(game) => format!(\"Zakuro - {} - {}\", game.name, game.system.status_line()),",
                "Some(game) => format!(\"{} — {}\", game.name, game.system.status_line()),",
            );
            src = src.replace(
                "Window::default_attributes().with_title(\"Zakuro\")",
                "Window::default_attributes().with_title(\"Pokémon Alpha Sapphire\")",
            );

            // Fournir `cheats` et `settings` directement au menu unifié `menus.game(...)`
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
                .with_title("Pokémon Saphir Alpha — Écran Tactile")
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

            fs::write(&lib_rs, src).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

/// Génère le fichier `settings.toml` pré-optimisé (par défaut : overlay classique `stacked` + `5:3` `console`).
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
        _ => "french",
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
        r#"# Configuration générée par BlueOrb
language = "{lang}"
renderer = "vulkan"
hardware_rasterizer = true
resolution = {res}
texture_packs = true
frame_interpolation = true
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
// BLUEORB_CONSOLE_THEME: Palette mate sombre sobre, typographie large et lisible (style SteamOS / RecompUI)
fn apply_blueorb_console_theme(ctx: &egui::Context) {
    use egui::{Color32, CornerRadius, CursorIcon, FontFamily, FontId, Shadow, Stroke, TextStyle, Theme};
    ctx.set_theme(Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(TextStyle::Small, FontId::new(12.5, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Body, FontId::new(15.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Button, FontId::new(14.5, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace));

        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(14.0, 8.0);
        style.spacing.window_margin = egui::Margin::same(18);

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.window_fill = Color32::from_rgb(20, 22, 28);
        v.panel_fill = Color32::from_rgb(20, 22, 28);
        v.faint_bg_color = Color32::from_rgb(28, 31, 39);
        v.extreme_bg_color = Color32::from_rgb(13, 15, 19);
        v.code_bg_color = Color32::from_rgb(15, 17, 22);
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(8);
        v.window_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 22));
        v.window_shadow = Shadow {
            offset: [0, 14],
            blur: 36,
            spread: 2,
            color: Color32::from_black_alpha(175),
        };
        v.popup_shadow = Shadow {
            offset: [0, 8],
            blur: 20,
            spread: 0,
            color: Color32::from_black_alpha(145),
        };
        v.slider_trailing_fill = true;
        v.interact_cursor = Some(CursorIcon::PointingHand);
        v.selection.bg_fill = Color32::from_rgb(37, 99, 235);
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);

        v.widgets.noninteractive.bg_fill = Color32::from_rgb(26, 29, 36);
        v.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(26, 29, 36);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 16));
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(235, 238, 244));
        v.widgets.noninteractive.corner_radius = CornerRadius::same(8);

        v.widgets.inactive.bg_fill = Color32::from_rgb(32, 36, 45);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(32, 36, 45);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 14));
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(235, 238, 244));
        v.widgets.inactive.corner_radius = CornerRadius::same(8);

        v.widgets.hovered.bg_fill = Color32::from_rgb(44, 50, 62);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(44, 50, 62);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 36));
        v.widgets.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        v.widgets.hovered.corner_radius = CornerRadius::same(8);

        v.widgets.active.bg_fill = Color32::from_rgb(37, 99, 235);
        v.widgets.active.weak_bg_fill = Color32::from_rgb(37, 99, 235);
        v.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(96, 165, 250));
        v.widgets.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        v.widgets.active.corner_radius = CornerRadius::same(8);

        v.widgets.open = v.widgets.hovered;
    });
}
"#;

const WIDE_CONSOLE_GAME_MENU_RS: &str = r#"    /// BLUEORB_WIDE_CONSOLE_MENU: Menu Console plein écran translucide basé sur egui::Panel (zéro bug d'alignement)
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
                .fill(Color32::from_rgba_unmultiplied(15, 17, 22, 224))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 24)))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(12, 7))
        };

        if let Some((text, since)) = &self.toast {
            if since.elapsed() < std::time::Duration::from_millis(2400) {
                egui::Area::new(egui::Id::new("toast"))
                    .order(egui::Order::Foreground)
                    .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 14.0))
                    .interactable(false)
                    .show(&ctx, |ui| {
                        hud_frame().show(ui, |ui| {
                            ui.label(RichText::new(text).size(14.0).color(Color32::from_rgb(240, 242, 246)));
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
                .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-12.0, 12.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    hud_frame().show(ui, |ui| {
                        ui.label(RichText::new("Avance rapide").size(13.0).monospace());
                    });
                });
        }
        if let Some(fps) = fps.filter(|_| !self.menu_open) {
            egui::Area::new(egui::Id::new("fps"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(12.0, 12.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    hud_frame().show(ui, |ui| {
                        ui.label(RichText::new(format!("{fps:.0} FPS")).size(13.0).monospace());
                    });
                });
        }
        if !self.menu_open {
            return actions;
        }

        let nav_tabs: [(u8, &str); 5] = [
            (0, "Partie"),
            (1, "Écrans & Format"),
            (2, "Graphismes & Son"),
            (3, "Contrôles"),
            (4, "Codes & Réseau"),
        ];

        let total_w = ui.available_width();
        let wide_mode = total_w >= 560.0;

        if wide_mode {
            let sidebar_w = 212.0;
            let sidebar_frame = egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(13, 15, 20, 248))
                .inner_margin(egui::Margin::symmetric(16, 22));

            egui::Panel::left("blueorb_pause_sidebar")
                .resizable(false)
                .exact_size(sidebar_w)
                .show_separator_line(true)
                .frame(sidebar_frame)
                .show(ui, |ui| {
                    let item_w = ui.available_width();

                    // Pied de la barre latérale ancré proprement en bas
                    egui::Panel::bottom("blueorb_pause_sidebar_footer")
                        .resizable(false)
                        .show_separator_line(false)
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 0, right: 0, top: 10, bottom: 0 }))
                        .show(ui, |ui| {
                            if console_primary_button(ui, item_w, 42.0, "Reprendre") {
                                actions.push(Action::Resume);
                            }
                            ui.add_space(8.0);
                            if console_danger_button(ui, item_w, 38.0, "Quitter le jeu") {
                                actions.push(Action::Quit);
                            }
                        });

                    // Haut + onglets de la barre latérale
                    ui.label(RichText::new(name).size(16.5).strong().color(Color32::WHITE));
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(if recompiled { "Exécution native AOT" } else { "Mode interprété" })
                            .size(12.5)
                            .color(Color32::from_rgb(132, 140, 156)),
                    );
                    ui.add_space(18.0);

                    ui.spacing_mut().item_spacing.y = 6.0;
                    for (idx, title) in nav_tabs {
                        if console_nav_button(ui, item_w, title, self.tab == idx) {
                            self.tab = idx;
                        }
                    }
                });
        } else {
            // Barre supérieure si la fenêtre est étroite (ex: 400px à l'échelle 1x)
            let top_frame = egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(13, 15, 20, 248))
                .inner_margin(egui::Margin::symmetric(12, 10));

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
                                    .size(13.0)
                                    .color(if sel { Color32::WHITE } else { Color32::from_rgb(172, 180, 194) }),
                            )
                            .fill(if sel { Color32::from_rgb(37, 99, 235) } else { Color32::from_rgb(24, 28, 36) });
                            if ui.add(btn).clicked() {
                                self.tab = idx;
                            }
                        }
                    });
                });
        }

        let central_margin_x: i8 = if wide_mode { 28 } else { 16 };
        let central_frame = egui::Frame::NONE
            .fill(Color32::from_rgba_unmultiplied(19, 22, 29, 242))
            .inner_margin(egui::Margin::symmetric(central_margin_x, 22));

        egui::CentralPanel::default().frame(central_frame).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("blueorb_pause_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let body_w = (ui.available_width() - 12.0).clamp(240.0, 720.0);
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
                    ui.add_space(16.0);
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
                // ONGLET 0 : PARTIE
                ui.label(RichText::new("Partie en cours").size(22.0).strong().color(Color32::WHITE));
                let layout_desc = match active_layout {
                    Screens::Stacked => "Écrans superposés (3DS)",
                    Screens::LargeTop => "Grand écran supérieur",
                    Screens::SideBySide => "Écrans côte à côte",
                    Screens::SeparateWindows => "Deux fenêtres indépendantes",
                    Screens::TopOnly => "Écran supérieur seul",
                    Screens::BottomOnly => "Écran tactile seul",
                };
                let ratio_desc = match active_top_shape {
                    TopShape::Console => "5:3 Original",
                    TopShape::Wide16x9 => "16:9 Large",
                    TopShape::Wide21x9 => "21:9 Ultra-Wide",
                    TopShape::Wide32x9 => "32:9 Super Large",
                };
                ui.label(
                    RichText::new(format!("{ratio_desc}  ·  {layout_desc}"))
                        .size(14.0)
                        .color(Color32::from_rgb(148, 156, 172)),
                );
                ui.add_space(18.0);

                if console_primary_button(ui, width, 46.0, "Reprendre la partie  (Échap)") {
                    actions.push(Action::Resume);
                }
                ui.add_space(10.0);

                let half_top = (width - 10.0) / 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if console_secondary_button(ui, half_top, 42.0, "Plein écran  (F11)", true) {
                        actions.push(Action::Fullscreen);
                    }
                    if console_secondary_button(ui, half_top, 42.0, "Redémarrer le jeu", true) {
                        actions.push(Action::Reset);
                    }
                });

                ui.add_space(18.0);

                console_card(ui, width, |ui, cw| {
                    let state_info = saved.unwrap_or("Emplacement vide");
                    ui.label(RichText::new("Sauvegardes rapides").size(16.5).strong().color(Color32::WHITE));
                    ui.label(
                        RichText::new(format!("Slot sélectionné : {}  —  {state_info}", self.slot))
                            .size(13.0)
                            .color(Color32::from_rgb(148, 156, 172)),
                    );
                    ui.add_space(10.0);

                    let slot_opts: [(u8, &str); 9] = [
                        (1, "1"), (2, "2"), (3, "3"), (4, "4"), (5, "5"), (6, "6"), (7, "7"), (8, "8"), (9, "9"),
                    ];
                    if let Some(s) = console_segmented(ui, cw, 40.0, self.slot, &slot_opts, "slots_seg") {
                        self.slot = s;
                    }

                    ui.add_space(12.0);
                    let half = (cw - 10.0) / 2.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        if console_secondary_button(ui, half, 42.0, &format!("Sauvegarder slot {}  (F5)", self.slot), true) {
                            actions.push(Action::SaveState);
                        }
                        if console_secondary_button(ui, half, 42.0, &format!("Charger slot {}  (F8)", self.slot), saved.is_some()) {
                            actions.push(Action::LoadState);
                        }
                    });
                });

                ui.add_space(16.0);
                let half_bot = (width - 10.0) / 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if console_secondary_button(ui, half_bot, 40.0, "Bibliothèque", true) {
                        actions.push(Action::Library);
                    }
                    if console_danger_button(ui, half_bot, 40.0, "Quitter le jeu") {
                        actions.push(Action::Quit);
                    }
                });

                for job in jobs.iter().filter(|job| !job.stage().finished()) {
                    ui.add_space(10.0);
                    ui.label(&job.name);
                    progress(ui, job);
                }
                if waiting > 0 {
                    ui.label(RichText::new(format!("{waiting} en attente")).weak());
                }
            }

            1 => {
                // ONGLET 1 : ÉCRANS & FORMAT ULTRA-WIDE
                ui.label(RichText::new("Disposition des écrans").size(22.0).strong().color(Color32::WHITE));
                ui.label(
                    RichText::new("Choisissez l'agencement ou détachez l'écran tactile sur une deuxième fenêtre (touche F7).")
                        .size(14.0)
                        .color(Color32::from_rgb(148, 156, 172)),
                );
                ui.add_space(14.0);

                // Grille 2x2 de grandes cartes horizontales lisibles (aucune coupure de texte !)
                let card_w = (width - 12.0) / 2.0;
                let row1 = [
                    (Screens::Stacked, "Superposé (Base)", "Écran haut au-dessus du bas", 0u8),
                    (Screens::SeparateWindows, "Deux fenêtres (F7)", "Détacher l'écran tactile", 3),
                ];
                let row2 = [
                    (Screens::LargeTop, "Grand écran", "Grand écran haut + petit bas", 1u8),
                    (Screens::SideBySide, "Côte à côte", "Gauche et droite alignés", 2),
                ];
                for row in [row1, row2] {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        for (lay, title, sub, icon_kind) in row {
                            let selected = active_layout == lay;
                            if console_screen_layout_card(ui, Vec2::new(card_w, 76.0), title, sub, icon_kind, selected) && !selected {
                                if lay == Screens::SeparateWindows {
                                    actions.push(Action::ToggleDualWindows);
                                } else {
                                    actions.push(Action::SetLayout(lay));
                                }
                            }
                        }
                    });
                    ui.add_space(10.0);
                }

                ui.add_space(10.0);
                ui.label(RichText::new("Format 3D & Ultra-Wide  (F6)").size(18.0).strong().color(Color32::WHITE));
                ui.label(
                    RichText::new("Élargit le champ de vision 3D horizontal sans étirer les modèles ni l'interface.")
                        .size(13.5)
                        .color(Color32::from_rgb(148, 156, 172)),
                );
                ui.add_space(8.0);

                let shape_opts = [
                    (TopShape::Console, "5:3 Base"),
                    (TopShape::Wide16x9, "16:9"),
                    (TopShape::Wide21x9, "21:9 Ultra-Wide"),
                    (TopShape::Wide32x9, "32:9"),
                ];
                if let Some(chosen) = console_segmented(ui, width, 42.0, active_top_shape, &shape_opts, "shape_seg") {
                    actions.push(Action::SetTopShape(chosen));
                }

                ui.add_space(16.0);
                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Taille de la fenêtre").size(15.0).strong().color(Color32::WHITE));
                    ui.add_space(6.0);
                    let scales = [(1u32, "1x"), (2, "2x"), (3, "3x"), (4, "4x"), (5, "5x")];
                    if let Some(s) = console_segmented(ui, cw, 38.0, settings.scale, &scales, "scale_seg") {
                        settings.scale = s;
                    }

                    ui.add_space(12.0);
                    ui.label(RichText::new("Filtre de mise à l'échelle").size(15.0).strong().color(Color32::WHITE));
                    ui.add_space(6.0);
                    let filters = [
                        (Filter::Sharp, "Net (Recommandé)"),
                        (Filter::Smooth, "Doux"),
                        (Filter::Pixels, "Pixels bruts"),
                    ];
                    if let Some(f) = console_segmented(ui, cw, 38.0, settings.filter, &filters, "filter_seg") {
                        settings.filter = f;
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);
                    console_toggle_row(
                        ui,
                        cw,
                        "Échelle entière stricte",
                        "Multiples entiers exacts avec bordures.",
                        &mut settings.integer_scale,
                    );
                });
            }

            2 => {
                // ONGLET 2 : GRAPHISMES & SON
                ui.label(RichText::new("Graphismes & Audio").size(22.0).strong().color(Color32::WHITE));
                ui.label(
                    RichText::new("Rendu 3D matériel, suppression des contours noirs HD et réglages audio.")
                        .size(14.0)
                        .color(Color32::from_rgb(148, 156, 172)),
                );
                ui.add_space(14.0);

                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Résolution interne 3D").size(15.5).strong().color(Color32::WHITE));
                    ui.label(
                        RichText::new("5x = QHD (2000×1200), 8x = 4K. S'applique au prochain démarrage ou redémarrage.")
                            .size(12.5)
                            .color(Color32::from_rgb(148, 156, 172)),
                    );
                    ui.add_space(8.0);
                    let res_opts = [
                        (1u32, "1x"),
                        (2, "2x"),
                        (3, "3x"),
                        (4, "4x"),
                        (5, "5x"),
                        (6, "6x"),
                        (8, "8x"),
                    ];
                    if let Some(r) = console_segmented(ui, cw, 40.0, settings.resolution, &res_opts, "res_seg") {
                        settings.resolution = r;
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(8.0);

                    console_toggle_row(
                        ui,
                        cw,
                        "Fluidité 60 FPS (Interpolation GPU)",
                        "Images intermédiaires à 60 FPS sans accélérer la vitesse du jeu.",
                        &mut settings.frame_interpolation,
                    );

                    if let Some(cheat_list) = cheats {
                        for (idx, (cname, enabled, notes, builtin)) in cheat_list.iter().enumerate() {
                            if !*builtin {
                                continue;
                            }
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(8.0);
                            let mut current = *enabled;
                            let title_fr = if cname.contains("No Outlines") {
                                "Supprimer les contours noirs (HD)"
                            } else if cname == "60 FPS" {
                                "Mode Turbo 2x (Vitesse doublée)"
                            } else {
                                cname.as_str()
                            };
                            let desc_fr = if cname.contains("No Outlines") {
                                "Retire en direct les traits noirs autour des personnages et Pokémon."
                            } else if cname == "60 FPS" {
                                "Double la vitesse d'exécution globale du moteur."
                            } else {
                                notes.as_str()
                            };
                            if console_toggle_row(ui, cw, title_fr, desc_fr, &mut current) {
                                actions.push(Action::Cheat(idx, current));
                            }
                        }
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);

                    console_toggle_row(
                        ui,
                        cw,
                        "Packs de textures personnalisés",
                        "Charge les textures placées dans le dossier mods.",
                        &mut settings.texture_packs,
                    );
                });

                ui.add_space(16.0);

                console_card(ui, width, |ui, cw| {
                    let pct = (settings.volume * 100.0).round() as i32;
                    ui.label(
                        RichText::new(format!("Volume principal  —  {pct}%"))
                            .size(15.5)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.add_space(6.0);
                    ui.spacing_mut().slider_width = cw;
                    ui.add(egui::Slider::new(&mut settings.volume, 0.0..=1.0).show_value(false));

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    console_toggle_row(ui, cw, "Couper le son", "Désactive la sortie audio.", &mut settings.mute);

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    console_toggle_row(
                        ui,
                        cw,
                        "Compteur d'images par seconde (FPS)",
                        "Affiche les FPS en haut à gauche de l'écran.",
                        &mut settings.show_fps,
                    );

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.label(RichText::new("Langue de la console (au redémarrage)").size(15.0).strong().color(Color32::WHITE));
                    ui.add_space(6.0);
                    let lang_opts = [
                        (Language::French, "Français"),
                        (Language::English, "English"),
                        (Language::Japanese, "日本語"),
                        (Language::German, "Deutsch"),
                        (Language::Spanish, "Español"),
                        (Language::Italian, "Italiano"),
                    ];
                    if let Some(l) = console_segmented(ui, cw, 38.0, settings.language, &lang_opts, "lang_seg") {
                        settings.language = l;
                    }
                });
            }

            3 => {
                // ONGLET 3 : CONTRÔLES
                ui.label(RichText::new("Contrôles").size(22.0).strong().color(Color32::WHITE));
                ui.label(
                    RichText::new("Cliquez sur une ligne pour réassigner une touche clavier ou un bouton manette.")
                        .size(14.0)
                        .color(Color32::from_rgb(148, 156, 172)),
                );
                ui.add_space(10.0);
                if console_secondary_button(ui, 220.0, 38.0, "Réinitialiser par défaut", true) {
                    settings.keys = Keys::default();
                    settings.pad = PadButtons::default();
                    self.rebinding = None;
                    self.rebinding_pad = None;
                }
                ui.add_space(14.0);

                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Clavier").size(16.5).strong().color(Color32::WHITE));
                    ui.add_space(8.0);
                    for (i, (kname, key)) in settings.keys.all_mut().into_iter().enumerate() {
                        let waiting_this = self.rebinding == Some(i);
                        let badge = if waiting_this { "Appuyez…".to_owned() } else { key_name(*key) };
                        if console_binding_row(ui, cw, kname, &badge, waiting_this) {
                            self.rebinding = Some(i);
                            self.rebinding_pad = None;
                        }
                    }
                });

                ui.add_space(16.0);

                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Manette").size(16.5).strong().color(Color32::WHITE));
                    ui.add_space(8.0);
                    for (i, (bname, button)) in settings.pad.all_mut().into_iter().enumerate() {
                        let waiting_this = self.rebinding_pad == Some(i);
                        let badge = if waiting_this { "Appuyez…" } else { button_name(*button) };
                        if console_binding_row(ui, cw, bname, badge, waiting_this) {
                            self.rebinding_pad = Some(i);
                            self.rebinding = None;
                        }
                    }
                });
            }

            _ => {
                // ONGLET 4 : CODES, MODS & RÉSEAU
                ui.label(RichText::new("Codes, Mods & Multijoueur").size(22.0).strong().color(Color32::WHITE));
                ui.label(
                    RichText::new("Salons sans-fil locaux, dossier mods, image caméra QR et codes Action Replay.")
                        .size(14.0)
                        .color(Color32::from_rgb(148, 156, 172)),
                );
                ui.add_space(14.0);

                let half = (width - 10.0) / 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if console_secondary_button(ui, half, 42.0, "Salons Multijoueur local", true) {
                        self.multiplayer_open = true;
                    }
                    if console_secondary_button(ui, half, 42.0, "Image Caméra / QR Code", true) {
                        actions.push(Action::ChooseCameraPicture);
                    }
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if let Some(pid) = program_id {
                        if console_secondary_button(ui, half, 42.0, "Ouvrir le dossier Mods", true) {
                            actions.push(Action::Mods(pid));
                        }
                    } else if console_secondary_button(ui, half, 42.0, "Ouvrir le dossier Cheats", true) {
                        actions.push(Action::CheatsFolder);
                    }
                    if console_secondary_button(ui, half, 42.0, "Copier le diagnostic", true) {
                        actions.push(Action::CopyReport);
                    }
                });

                ui.add_space(16.0);

                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Codes Action Replay").size(16.5).strong().color(Color32::WHITE));
                    ui.add_space(6.0);
                    let btn_half = (cw - 10.0) / 2.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        if console_secondary_button(ui, btn_half, 36.0, "Recharger le fichier", true) {
                            actions.push(Action::ReloadCheats);
                        }
                        if console_secondary_button(ui, btn_half, 36.0, "Ouvrir le dossier cheats", true) {
                            actions.push(Action::CheatsFolder);
                        }
                    });
                    ui.add_space(10.0);

                    if let Some(cheat_list) = cheats {
                        let user_cheats: Vec<_> = cheat_list.iter().enumerate().filter(|(_, c)| !c.3).collect();
                        if user_cheats.is_empty() {
                            ui.label(
                                RichText::new("Aucun code personnalisé enregistré.")
                                    .size(13.5)
                                    .color(Color32::from_rgb(148, 156, 172)),
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

                    ui.add_space(10.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.cheat_name)
                            .hint_text("Nom du code")
                            .margin(egui::Margin::symmetric(10, 8))
                            .desired_width(cw),
                    );
                    ui.add_space(6.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.cheat_code)
                            .hint_text("00000000 00000000")
                            .margin(egui::Margin::symmetric(10, 8))
                            .desired_rows(3)
                            .desired_width(cw)
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.add_space(8.0);
                    if console_primary_button(ui, 190.0, 38.0, "Ajouter le code") && !self.cheat_code.trim().is_empty() {
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
fn console_nav_button(ui: &mut egui::Ui, width: f32, label: &str, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 42.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered();
        let bg = if active {
            Color32::from_rgb(30, 35, 46)
        } else if hovered {
            Color32::from_rgb(22, 26, 34)
        } else {
            Color32::TRANSPARENT
        };
        if bg != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(8), bg);
        }
        if active {
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 4.0, rect.top() + 10.0),
                Vec2::new(3.5, rect.height() - 20.0),
            );
            ui.painter().rect_filled(bar, egui::CornerRadius::same(2), Color32::from_rgb(59, 130, 246));
        }
        let text_col = if active || hovered {
            Color32::WHITE
        } else {
            Color32::from_rgb(172, 180, 194)
        };
        ui.painter().text(
            egui::pos2(rect.left() + 16.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(14.5),
            text_col,
        );
    }
    resp.clicked()
}

fn console_primary_button(ui: &mut egui::Ui, width: f32, height: f32, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if resp.is_pointer_button_down_on() {
            Color32::from_rgb(29, 78, 216)
        } else if resp.hovered() {
            Color32::from_rgb(59, 130, 246)
        } else {
            Color32::from_rgb(37, 99, 235)
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(9), fill);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(14.5),
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
            Color32::from_rgb(24, 27, 34)
        } else if resp.is_pointer_button_down_on() {
            Color32::from_rgb(48, 55, 68)
        } else if resp.hovered() {
            Color32::from_rgb(40, 46, 58)
        } else {
            Color32::from_rgb(30, 34, 44)
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(9), fill);
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(9),
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 18)),
            egui::StrokeKind::Inside,
        );
        let col = if enabled {
            Color32::from_rgb(236, 239, 245)
        } else {
            Color32::from_rgb(104, 112, 126)
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(14.0),
            col,
        );
    }
    enabled && resp.clicked()
}

fn console_danger_button(ui: &mut egui::Ui, width: f32, height: f32, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if resp.hovered() {
            Color32::from_rgb(76, 30, 36)
        } else {
            Color32::from_rgb(48, 22, 26)
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(9), fill);
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(9),
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(248, 113, 113, 64)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(14.0),
            Color32::from_rgb(252, 165, 165),
        );
    }
    resp.clicked()
}

fn console_card(ui: &mut egui::Ui, width: f32, body: impl FnOnce(&mut egui::Ui, f32)) {
    egui::Frame::NONE
        .fill(Color32::from_rgb(26, 29, 38))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 18)))
        .corner_radius(egui::CornerRadius::same(11))
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            let cw = (width - 38.0).max(160.0);
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
        ui.painter().rect_filled(outer_rect, egui::CornerRadius::same(9), Color32::from_rgb(14, 16, 21));
        ui.painter().rect_stroke(
            outer_rect,
            egui::CornerRadius::same(9),
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 16)),
            egui::StrokeKind::Inside,
        );
    }
    let pad = 4.0;
    let inner = outer_rect.shrink(pad);
    let n = options.len().max(1) as f32;
    let gap = 4.0;
    let item_w = ((inner.width() - gap * (n - 1.0)) / n).max(12.0);

    for (idx, &(val, label)) in options.iter().enumerate() {
        let x0 = inner.left() + (idx as f32) * (item_w + gap);
        let item_rect = egui::Rect::from_min_size(egui::pos2(x0, inner.top()), Vec2::new(item_w, inner.height()));
        let resp = ui.interact(item_rect, ui.make_persistent_id((id_salt, idx)), egui::Sense::click());
        let selected = val == current;
        if ui.is_rect_visible(item_rect) {
            if selected {
                ui.painter().rect_filled(item_rect, egui::CornerRadius::same(6), Color32::from_rgb(37, 99, 235));
            } else if resp.hovered() {
                ui.painter().rect_filled(item_rect, egui::CornerRadius::same(6), Color32::from_rgb(28, 32, 42));
            }
            let col = if selected || resp.hovered() {
                Color32::WHITE
            } else {
                Color32::from_rgb(172, 180, 194)
            };
            ui.painter().text(
                item_rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(13.5),
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
    let row_h = if subtitle.is_empty() { 34.0 } else { 46.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, row_h), egui::Sense::click());
    let mut changed = false;
    if resp.clicked() {
        *value = !*value;
        changed = true;
    }
    if ui.is_rect_visible(rect) {
        let switch_w = 46.0;
        let switch_h = 26.0;
        let switch_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - switch_w, rect.center().y - switch_h * 0.5),
            Vec2::new(switch_w, switch_h),
        );
        let track = if *value {
            Color32::from_rgb(37, 99, 235)
        } else if resp.hovered() {
            Color32::from_rgb(56, 62, 76)
        } else {
            Color32::from_rgb(42, 47, 58)
        };
        ui.painter().rect_filled(switch_rect, egui::CornerRadius::same(13), track);
        let knob_x = if *value { switch_rect.right() - 13.0 } else { switch_rect.left() + 13.0 };
        ui.painter().circle_filled(egui::pos2(knob_x, switch_rect.center().y), 9.5, Color32::WHITE);

        let text_clip = egui::Rect::from_min_max(rect.min, egui::pos2(switch_rect.left() - 14.0, rect.bottom()));
        let tp = ui.painter().with_clip_rect(text_clip);
        if subtitle.is_empty() {
            tp.text(
                egui::pos2(rect.left(), rect.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(14.5),
                Color32::from_rgb(240, 242, 246),
            );
        } else {
            tp.text(
                egui::pos2(rect.left(), rect.top() + 12.0),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(14.5),
                Color32::from_rgb(240, 242, 246),
            );
            tp.text(
                egui::pos2(rect.left(), rect.top() + 33.0),
                egui::Align2::LEFT_CENTER,
                subtitle,
                egui::FontId::proportional(12.5),
                Color32::from_rgb(142, 150, 166),
            );
        }
    }
    changed
}

fn console_binding_row(ui: &mut egui::Ui, width: f32, label: &str, badge: &str, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 36.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(6), Color32::from_rgb(32, 36, 46));
        }
        ui.painter().text(
            egui::pos2(rect.left() + 6.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(14.0),
            Color32::from_rgb(228, 232, 240),
        );
        let badge_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 134.0, rect.center().y - 14.0),
            Vec2::new(130.0, 28.0),
        );
        let fill = if active {
            Color32::from_rgb(37, 99, 235)
        } else {
            Color32::from_rgb(16, 19, 25)
        };
        ui.painter().rect_filled(badge_rect, egui::CornerRadius::same(6), fill);
        ui.painter().rect_stroke(
            badge_rect,
            egui::CornerRadius::same(6),
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 18)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            badge_rect.center(),
            egui::Align2::CENTER_CENTER,
            badge,
            egui::FontId::monospace(13.0),
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
            Color32::from_rgb(26, 36, 56)
        } else if resp.hovered() {
            Color32::from_rgb(33, 38, 49)
        } else {
            Color32::from_rgb(26, 29, 38)
        };
        let border = if selected {
            egui::Stroke::new(1.5, Color32::from_rgb(59, 130, 246))
        } else if resp.hovered() {
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 42))
        } else {
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 18))
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(10), bg);
        ui.painter().rect_stroke(rect, egui::CornerRadius::same(10), border, egui::StrokeKind::Inside);

        // Icône géométrique dans les 68 premiers pixels à gauche de la carte horizontale
        let cx = rect.left() + 36.0;
        let cy = rect.center().y;
        let top_fill = if selected {
            Color32::from_rgb(96, 165, 250)
        } else {
            Color32::from_rgb(138, 148, 168)
        };
        let bot_fill = if selected {
            Color32::from_rgba_unmultiplied(96, 165, 250, 165)
        } else {
            Color32::from_rgba_unmultiplied(138, 148, 168, 145)
        };

        match kind {
            0 => {
                let r_top = egui::Rect::from_center_size(egui::pos2(cx, cy - 10.0), Vec2::new(34.0, 17.0));
                let r_bot = egui::Rect::from_center_size(egui::pos2(cx, cy + 10.0), Vec2::new(26.0, 15.0));
                ui.painter().rect_filled(r_top, egui::CornerRadius::same(3), top_fill);
                ui.painter().rect_filled(r_bot, egui::CornerRadius::same(3), bot_fill);
            }
            1 => {
                let r_top = egui::Rect::from_center_size(egui::pos2(cx - 8.0, cy), Vec2::new(32.0, 22.0));
                let r_bot = egui::Rect::from_center_size(egui::pos2(cx + 16.0, cy + 5.0), Vec2::new(14.0, 12.0));
                ui.painter().rect_filled(r_top, egui::CornerRadius::same(3), top_fill);
                ui.painter().rect_filled(r_bot, egui::CornerRadius::same(2), bot_fill);
            }
            2 => {
                let r_top = egui::Rect::from_center_size(egui::pos2(cx - 12.0, cy), Vec2::new(24.0, 16.0));
                let r_bot = egui::Rect::from_center_size(egui::pos2(cx + 13.0, cy), Vec2::new(20.0, 16.0));
                ui.painter().rect_filled(r_top, egui::CornerRadius::same(3), top_fill);
                ui.painter().rect_filled(r_bot, egui::CornerRadius::same(3), bot_fill);
            }
            _ => {
                let w1 = egui::Rect::from_center_size(egui::pos2(cx - 8.0, cy - 5.0), Vec2::new(30.0, 19.0));
                let w2 = egui::Rect::from_center_size(egui::pos2(cx + 10.0, cy + 6.0), Vec2::new(24.0, 17.0));
                ui.painter().rect_filled(w1, egui::CornerRadius::same(3), top_fill);
                ui.painter().rect_filled(w2.expand(2.0), egui::CornerRadius::same(4), bg);
                ui.painter().rect_filled(w2, egui::CornerRadius::same(3), bot_fill);
            }
        }

        let text_clip = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 72.0, rect.top() + 4.0),
            egui::pos2(rect.right() - 10.0, rect.bottom() - 4.0),
        );
        let tp = ui.painter().with_clip_rect(text_clip);
        tp.text(
            egui::pos2(rect.left() + 72.0, rect.center().y - 9.0),
            egui::Align2::LEFT_CENTER,
            title,
            egui::FontId::proportional(14.5),
            Color32::WHITE,
        );
        tp.text(
            egui::pos2(rect.left() + 72.0, rect.center().y + 11.0),
            egui::Align2::LEFT_CENTER,
            subtitle,
            egui::FontId::proportional(12.0),
            Color32::from_rgb(142, 150, 166),
        );
    }
    resp.clicked()
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
