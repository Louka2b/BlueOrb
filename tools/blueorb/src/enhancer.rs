// Patchs d'améliorations Remaster HD, Ultra-Wide (16:9 / 21:9 / 32:9), Double Fenêtre détachable (Haut + Bas)
// et Thème / Menu "BlueOrb Saphir Abyssal" appliqués automatiquement aux sources `build/recomp_sdk/zakuro`
// et `build/recomp_sdk/3dsrecomp` sur Linux (x86_64), macOS (ARM64 / Intel) et Windows (x86_64).

use std::fs;
use std::path::Path;

pub fn apply_sdk_enhancements(recomp_repo: &Path, zakuro_repo: &Path) -> Result<(), String> {
    patch_recomp_incremental(recomp_repo)?;
    patch_zakuro_oras_enhancements(zakuro_repo)?;
    patch_zakuro_gpu_ultrawide_upscaler_and_dual_vulkan(zakuro_repo)?;
    patch_zakuro_frontend_dual_window_ultrawide_and_sapphire_ui(zakuro_repo)?;
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
    // 3a. `zakuro-gpu/src/lib.rs` : variable atomique globale pour le facteur Hor+ Ultra-Wide de l'écran supérieur
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

    // 3b. `zakuro-gpu/src/raster.rs` : expansion Hor+ Ultra-Wide dans `fn place` (couvre CPU/Cranelift + 60 FPS Matrix Interpolation)
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

    // 3c. `zakuro-gpu/src/shader/glsl.rs` : expansion Hor+ Ultra-Wide pour les GPU dédiés (Windows/Linux NVIDIA/AMD)
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

    // 3d. `zakuro-gpu/src/raster/hardware.rs` : Upscaler 4x Smooth-xBR + Anisotropie 16x + passage de `aspect_scale` dans `pad[0]`
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

    // 3e. `zakuro-gpu/src/backend/vulkan.rs` : ajout de `VulkanPresenter::new_secondary` (partage `Arc<SharedDevice>` + PresentMode non-bloquant)
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

/// 4. Patchs du frontend `zakuro` (`gui.rs`, `menus.rs`, `settings.rs`, `present.rs`, `lib.rs`) :
///    - Thème "BlueOrb Saphir Abyssal" moderne (glassmorphism bleu océan/saphir, coins arrondis, ombres douces, accents cyan)
///    - Menu Pause (`Échap`) entièrement repensé avec barre d'actions rapides 1-clic pour :
///        * Détacher / Regrouper les 2 écrans (`F7`)
///        * Choisir la disposition 1 fenêtre (`Superposé (Base)`, `Grand Haut`, `Côte à côte`)
///        * Activer le vrai rendu 3D Ultra-Wide (`5:3 Base`, `16:9`, `21:9 UW`, `32:9` — `F6`)
///    - Par défaut : disposition de base `Screens::Stacked` + format `TopShape::Console` (`5:3`)
fn patch_zakuro_frontend_dual_window_ultrawide_and_sapphire_ui(zakuro_repo: &Path) -> Result<(), String> {
    let zakuro_src = zakuro_repo.join("crates").join("zakuro").join("src");

    // 4a. `gui.rs` : Thème visuel "BlueOrb Saphir Abyssal"
    let gui_rs = zakuro_src.join("gui.rs");
    if gui_rs.exists() {
        let mut src = fs::read_to_string(&gui_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_SAPPHIRE_THEME") {
            src = src.replace(
                "        ctx.set_fonts(fonts(system_fonts()));",
                "        ctx.set_fonts(fonts(system_fonts()));\n        apply_blueorb_sapphire_theme(&ctx);",
            );
            src.push_str(SAPPHIRE_THEME_FN_RS);
            fs::write(&gui_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // 4b. `settings.rs` : `ZAKURO_SETTINGS_PATH`, `Screens::SeparateWindows` et `TopShape::Wide32x9`
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
                "Screens::LargeTop => \"Big top screen, small bottom one\",\n            Screens::SeparateWindows => \"Two separate windows (Top + Bottom)\",",
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
                "TopShape::Wide21x9 => \"21:9 Ultra-Wide\",\n            TopShape::Wide32x9 => \"32:9 Super Ultra-Wide\",",
            );
            src = src.replace(
                "TopShape::Wide21x9 => 560.0,",
                "TopShape::Wide21x9 => 560.0,\n            TopShape::Wide32x9 => 240.0 * 32.0 / 9.0,",
            );
        }
        fs::write(&settings_rs, src).map_err(|e| e.to_string())?;
    }

    // 4c. `menus.rs` : Nouveau menu Pause `Échap` élégant + contrôles directs Détacher Écrans & Ultra-Wide
    let menus_rs = zakuro_src.join("menus.rs");
    if menus_rs.exists() {
        let mut src = fs::read_to_string(&menus_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_SAPPHIRE_GAME_MENU") {
            src = src.replace(
                "    CheatsFolder,\n}",
                "    CheatsFolder,\n    ToggleDualWindows,\n    SetLayout(Screens),\n    SetTopShape(TopShape),\n}",
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
            src = src.replace(old_game_fn, NEW_SAPPHIRE_GAME_MENU_RS);
            fs::write(&menus_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // 4d. `present.rs` : ajout de `Backend::create_secondary`
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

    // 4e. `lib.rs` : gestion de la 2e fenêtre détachable (`last_combined_layout` = `Stacked` par défaut), actions du nouveau menu, et raccourcis `F6` / `F7`
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

            // Mémoriser `last_combined_layout` dans `set_layout` + ajouter `toggle_dual_windows` et `set_top_shape`
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
                "Écrans détachés en 2 fenêtres (F7)".to_owned()
            } else {
                format!("Écrans regroupés : {}", target.name())
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
        self.menus.toast = Some((format!("Format 3D : {}", shape.name()), Instant::now()));
    }

    fn set_layout(&mut self, layout: Screens) {
        if layout != Screens::SeparateWindows {
            self.last_combined_layout = layout;
        }"#;
            src = src.replace(set_layout_needle, set_layout_rep);

            // Événements de la 2e fenêtre dans `window_event`
            let win_ev_needle = "    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {";
            let win_ev_rep = r#"    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.bottom_window.as_ref().is_some_and(|w| w.id() == id) {
            match event {
                WindowEvent::CloseRequested => {
                    // Fermer la fenêtre tactile rattache les 2 écrans dans la disposition précédente (Stacked par défaut) !
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

            // Raccourcis clavier F6 (cycle Ultra-Wide 5:3 / 16:9 / 21:9 / 32:9) et F7 (Détacher / Regrouper les 2 écrans)
            let key_needle = "                KeyCode::F9 => self.set_layout(self.active.layout.next()),";
            let key_rep = r#"                KeyCode::F6 => {
                    let shapes = settings::TopShape::ALL;
                    let idx = shapes.iter().position(|&s| s == self.active.top_shape).unwrap_or(0);
                    self.set_top_shape(shapes[(idx + 1) % shapes.len()]);
                }
                KeyCode::F7 => self.toggle_dual_windows(),
                KeyCode::F9 => self.set_layout(self.active.layout.next()),"#;
            src = src.replace(key_needle, key_rep);

            // Tactile sur la 2e fenêtre
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

            // `apply_layout` + initialisation `resumed`
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

            // Actions du menu Pause (`ToggleDualWindows`, `SetLayout`, `SetTopShape`)
            src = src.replace(
                "            Action::Quit => event_loop.exit(),",
                "            Action::ToggleDualWindows => self.toggle_dual_windows(),\n            Action::SetLayout(layout) => {\n                self.set_layout(layout);\n                self.menus.toast = Some((format!(\"Disposition : {}\", layout.name()), Instant::now()));\n            }\n            Action::SetTopShape(shape) => self.set_top_shape(shape),\n            Action::Quit => event_loop.exit(),",
            );

            // Titre de fenêtre épuré
            src = src.replace(
                "Some(game) => format!(\"Zakuro - {} - {}\", game.name, game.system.status_line()),",
                "Some(game) => format!(\"{} — BlueOrb ({})\", game.name, game.system.status_line()),",
            );
            src = src.replace(
                "Window::default_attributes().with_title(\"Zakuro\")",
                "Window::default_attributes().with_title(\"Pokémon Alpha Sapphire — BlueOrb\")",
            );

            // Passage de `active_layout` et `active_top_shape` à `menus.game(...)`
            src = src.replace(
                "        let (menus, library, settings, jobs, waiting) = (&mut self.menus, &self.library, &mut self.settings, &self.jobs, &self.waiting);",
                "        let (active_layout, active_top_shape) = (self.active.layout, self.active.top_shape);\n        let (menus, library, settings, jobs, waiting) = (&mut self.menus, &self.library, &mut self.settings, &self.jobs, &self.waiting);",
            );
            src = src.replace(
                "menus.game(ui, name, program_id, show_fps.then_some(*fps), *recompiled, fast, jobs, waiting.len(), saved.as_deref())",
                "menus.game(ui, name, program_id, show_fps.then_some(*fps), *recompiled, fast, jobs, waiting.len(), saved.as_deref(), active_layout, active_top_shape)",
            );

            // Synchronisation dynamique (création / fermeture) + rendu zéro-copie de la 2e fenêtre dans `present`
            let present_needle = "    fn present(&mut self, event_loop: &ActiveEventLoop) {\n        let overlay = self.interface(event_loop);";
            let present_rep = r#"    fn sync_bottom_window(&mut self, event_loop: &ActiveEventLoop) {
        let wants_second = self.layout == Screens::SeparateWindows && self.game.is_some();
        if wants_second && self.bottom_window.is_none() {
            let bot_scale = self.scale.clamp(2, 4);
            let bot_size = winit::dpi::LogicalSize::new(320 * bot_scale, 240 * bot_scale);
            let attrs = Window::default_attributes()
                .with_title("Pokémon Saphir Alpha — Écran Tactile (PokéMulti-Navi)")
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
        r#"# Configuration Remaster générée par BlueOrb
# Par défaut : disposition de base 3DS (stacked, 5:3).
# En jeu : Échap ouvre le menu BlueOrb (Détacher les 2 écrans, Ultra-Wide 16:9 / 21:9 / 32:9, HD, Sauvegardes)
# Raccourcis : F6 (Cycle 5:3 / 16:9 / 21:9 / 32:9) | F7 (Détacher / Regrouper les 2 écrans) | F11 (Plein écran)
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

const SAPPHIRE_THEME_FN_RS: &str = r#"
// BLUEORB_SAPPHIRE_THEME: Thème Glassmorphism "Saphir Abyssal" pour l'interface en jeu
fn apply_blueorb_sapphire_theme(ctx: &egui::Context) {
    use egui::{Color32, CornerRadius, CursorIcon, Shadow, Stroke, Theme};
    ctx.set_theme(Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(7.0, 5.5);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.window_margin = egui::Margin::same(14);

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.window_fill = Color32::from_rgba_unmultiplied(9, 17, 34, 242);
        v.panel_fill = Color32::from_rgba_unmultiplied(11, 21, 42, 242);
        v.faint_bg_color = Color32::from_rgba_unmultiplied(18, 35, 66, 185);
        v.extreme_bg_color = Color32::from_rgb(6, 12, 24);
        v.code_bg_color = Color32::from_rgb(12, 24, 46);
        v.window_corner_radius = CornerRadius::same(14);
        v.menu_corner_radius = CornerRadius::same(10);
        v.window_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(68, 166, 255, 105));
        v.window_shadow = Shadow {
            offset: [0, 10],
            blur: 28,
            spread: 2,
            color: Color32::from_black_alpha(150),
        };
        v.popup_shadow = Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 1,
            color: Color32::from_black_alpha(130),
        };
        v.slider_trailing_fill = true;
        v.interact_cursor = Some(CursorIcon::PointingHand);
        v.selection.bg_fill = Color32::from_rgb(26, 118, 220);
        v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(240, 250, 255));
        v.hyperlink_color = Color32::from_rgb(94, 198, 255);

        v.widgets.noninteractive.bg_fill = Color32::from_rgba_unmultiplied(14, 26, 50, 220);
        v.widgets.noninteractive.weak_bg_fill = Color32::from_rgba_unmultiplied(14, 26, 50, 180);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(58, 126, 208, 70));
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(224, 236, 252));
        v.widgets.noninteractive.corner_radius = CornerRadius::same(8);

        v.widgets.inactive.bg_fill = Color32::from_rgb(19, 36, 66);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(19, 36, 66);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(74, 150, 240, 68));
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(228, 240, 255));
        v.widgets.inactive.corner_radius = CornerRadius::same(8);

        v.widgets.hovered.bg_fill = Color32::from_rgb(28, 62, 114);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(28, 62, 114);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(96, 192, 255));
        v.widgets.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        v.widgets.hovered.corner_radius = CornerRadius::same(8);

        v.widgets.active.bg_fill = Color32::from_rgb(32, 122, 224);
        v.widgets.active.weak_bg_fill = Color32::from_rgb(32, 122, 224);
        v.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(150, 220, 255));
        v.widgets.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        v.widgets.active.corner_radius = CornerRadius::same(8);

        v.widgets.open = v.widgets.hovered;
    });
}
"#;

const NEW_SAPPHIRE_GAME_MENU_RS: &str = r#"    /// BLUEORB_SAPPHIRE_GAME_MENU: Overlay Pause moderne avec contrôles directs Détacher 2 Écrans & Ultra-Wide
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
        active_layout: Screens,
        active_top_shape: TopShape,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let ctx = ui.ctx().clone();

        let pill_frame = || {
            egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(8, 18, 36, 228))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(82, 182, 255, 130)))
                .corner_radius(egui::CornerRadius::same(18))
                .inner_margin(egui::Margin::symmetric(12, 6))
        };

        if let Some((text, since)) = &self.toast {
            if since.elapsed() < std::time::Duration::from_millis(2600) {
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 12.0))
                    .interactable(false)
                    .show(&ctx, |ui| {
                        pill_frame().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("◆").color(Color32::from_rgb(76, 194, 255)).strong());
                                ui.label(RichText::new(text).color(Color32::from_rgb(235, 245, 255)).strong());
                            });
                        });
                    });
                ctx.request_repaint();
            } else {
                self.toast = None;
            }
        }
        if fast {
            egui::Area::new(egui::Id::new("fast forward"))
                .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-10.0, 10.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    pill_frame().show(ui, |ui| {
                        ui.label(RichText::new("»» TURBO").color(Color32::from_rgb(110, 215, 255)).monospace().strong());
                    });
                });
        }
        if let Some(fps) = fps {
            egui::Area::new(egui::Id::new("fps"))
                .fixed_pos(egui::pos2(10.0, 10.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    pill_frame().show(ui, |ui| {
                        ui.label(RichText::new(format!("{fps:.0} FPS")).color(Color32::from_rgb(130, 225, 195)).monospace().strong());
                    });
                });
        }
        if !self.menu_open {
            return actions;
        }

        // Voile sombre Saphir en arrière-plan (cliquer en dehors du menu reprend la partie si aucune sous-fenêtre n'est ouverte)
        let subwindow_open = self.settings_open || self.game_settings.is_some() || self.cheats_open || self.multiplayer_open;
        let screen_rect = ctx.content_rect();
        egui::Area::new(egui::Id::new("blueorb_pause_backdrop"))
            .order(egui::Order::Background)
            .fixed_pos(screen_rect.min)
            .show(&ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen_rect.size(), egui::Sense::click());
                ui.painter().rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(4, 9, 20, 148));
                if resp.clicked() && !subwindow_open {
                    actions.push(Action::Resume);
                }
            });

        let card_width = 348.0f32.min((screen_rect.width() - 24.0).max(220.0));
        let max_scroll_h = (screen_rect.height() - 40.0).max(150.0);

        let modal_frame = egui::Frame::NONE
            .fill(Color32::from_rgba_unmultiplied(9, 18, 36, 246))
            .stroke(egui::Stroke::new(1.2, Color32::from_rgba_unmultiplied(78, 178, 255, 135)))
            .corner_radius(egui::CornerRadius::same(16))
            .shadow(egui::Shadow {
                offset: [0, 12],
                blur: 32,
                spread: 3,
                color: Color32::from_black_alpha(165),
            })
            .inner_margin(egui::Margin::same(16));

        let section_frame = || {
            egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(15, 29, 56, 210))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(64, 142, 230, 65)))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::symmetric(10, 9))
        };

        egui::Window::new("BlueOrb Pause")
            .id(egui::Id::new("blueorb_pause_modal"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .frame(modal_frame)
            .fixed_size([card_width, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(&ctx, |ui| {
                egui::ScrollArea::vertical().max_height(max_scroll_h).show(ui, |ui| {
                    ui.set_width(card_width);

                    // 1. En-tête Saphir + Badge AOT Natif
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(name).size(16.5).strong().color(Color32::WHITE));
                            let shape_short = match active_top_shape {
                                TopShape::Console => "5:3 Base",
                                TopShape::Wide16x9 => "16:9 Widescreen",
                                TopShape::Wide21x9 => "21:9 Ultra-Wide",
                                TopShape::Wide32x9 => "32:9 Super UW",
                            };
                            let layout_short = match active_layout {
                                Screens::Stacked => "Écrans superposés",
                                Screens::LargeTop => "Grand écran haut",
                                Screens::SideBySide => "Côte à côte",
                                Screens::SeparateWindows => "2 Fenêtres détachées",
                                Screens::TopOnly => "Écran haut seul",
                                Screens::BottomOnly => "Écran bas seul",
                            };
                            ui.label(
                                RichText::new(format!("{shape_short}  •  {layout_short}"))
                                    .size(11.5)
                                    .color(Color32::from_rgb(138, 184, 232)),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                            let badge_txt = if recompiled { "◆ AOT NATIF" } else { "INTERPRÉTÉ" };
                            let badge_col = if recompiled {
                                Color32::from_rgb(92, 206, 255)
                            } else {
                                Color32::from_rgb(220, 180, 100)
                            };
                            egui::Frame::NONE
                                .fill(Color32::from_rgba_unmultiplied(20, 56, 102, 210))
                                .stroke(egui::Stroke::new(1.0, badge_col.gamma_multiply(0.55)))
                                .corner_radius(egui::CornerRadius::same(12))
                                .inner_margin(egui::Margin::symmetric(8, 3))
                                .show(ui, |ui| {
                                    ui.label(RichText::new(badge_txt).size(10.5).strong().color(badge_col));
                                });
                        });
                    });

                    ui.add_space(6.0);

                    // 2. Bouton principal Reprendre
                    let resume_btn = egui::Button::new(
                        RichText::new("▶   Reprendre la partie").size(14.0).strong().color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(26, 116, 222))
                    .stroke(egui::Stroke::new(1.0, Color32::from_rgb(112, 198, 255)))
                    .corner_radius(egui::CornerRadius::same(10));
                    if ui.add_sized([card_width, 34.0], resume_btn).clicked() {
                        actions.push(Action::Resume);
                    }

                    ui.add_space(6.0);

                    // 3. Carte ÉCRANS & ULTRA-WIDE
                    section_frame().show(ui, |ui| {
                        let inner_w = (card_width - 22.0).max(180.0);
                        ui.set_width(inner_w);
                        ui.label(
                            RichText::new("AFFICHAGE, DOUBLE ÉCRAN & ULTRA-WIDE")
                                .size(10.5)
                                .strong()
                                .color(Color32::from_rgb(110, 184, 250)),
                        );
                        ui.add_space(3.0);

                        // Bouton 1-clic Détacher / Regrouper les 2 écrans
                        let detached = active_layout == Screens::SeparateWindows;
                        let detach_label = if detached {
                            "⊞  Regrouper les 2 écrans en 1 fenêtre  (F7)"
                        } else {
                            "⧉  Détacher les 2 écrans (2 fenêtres · F7)"
                        };
                        let detach_btn = egui::Button::new(
                            RichText::new(detach_label)
                                .size(12.5)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(if detached {
                            Color32::from_rgb(18, 128, 168)
                        } else {
                            Color32::from_rgb(24, 52, 96)
                        })
                        .stroke(egui::Stroke::new(
                            1.0,
                            if detached {
                                Color32::from_rgb(110, 230, 255)
                            } else {
                                Color32::from_rgb(76, 158, 245)
                            },
                        ))
                        .corner_radius(egui::CornerRadius::same(8));

                        if ui
                            .add_sized([inner_w, 30.0], detach_btn)
                            .on_hover_text("Ouvre l'écran tactile PokéMulti-Navi dans une seconde fenêtre indépendante, ou regroupe les deux écrans.")
                            .clicked()
                        {
                            actions.push(Action::ToggleDualWindows);
                        }

                        ui.add_space(4.0);

                        // Sélecteur rapide de disposition 1 fenêtre
                        let layouts_row = [
                            (Screens::Stacked, "Classique"),
                            (Screens::LargeTop, "Grand Haut"),
                            (Screens::SideBySide, "Côte à côte"),
                        ];
                        let pill_w3 = (inner_w - 12.0) / 3.0;
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for (lay, label) in layouts_row {
                                let selected = active_layout == lay;
                                let btn = egui::Button::new(
                                    RichText::new(label).size(11.5).strong().color(if selected {
                                        Color32::WHITE
                                    } else {
                                        Color32::from_rgb(195, 215, 240)
                                    }),
                                )
                                .fill(if selected {
                                    Color32::from_rgb(32, 118, 218)
                                } else {
                                    Color32::from_rgb(16, 30, 56)
                                })
                                .stroke(egui::Stroke::new(
                                    1.0,
                                    if selected {
                                        Color32::from_rgb(125, 205, 255)
                                    } else {
                                        Color32::from_rgba_unmultiplied(64, 130, 210, 60)
                                    },
                                ));
                                if ui.add_sized([pill_w3, 24.0], btn).clicked() && !selected {
                                    actions.push(Action::SetLayout(lay));
                                }
                            }
                        });

                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("Format 3D & Ultra-Wide (F6)")
                                .size(11.0)
                                .color(Color32::from_rgb(168, 198, 232)),
                        );

                        // Sélecteur rapide Ultra-Wide 5:3 / 16:9 / 21:9 / 32:9
                        let shapes_row = [
                            (TopShape::Console, "5:3 Base"),
                            (TopShape::Wide16x9, "16:9"),
                            (TopShape::Wide21x9, "21:9 UW"),
                            (TopShape::Wide32x9, "32:9"),
                        ];
                        let pill_w4 = (inner_w - 18.0) / 4.0;
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for (sh, label) in shapes_row {
                                let selected = active_top_shape == sh;
                                let btn = egui::Button::new(
                                    RichText::new(label).size(11.5).strong().color(if selected {
                                        Color32::WHITE
                                    } else {
                                        Color32::from_rgb(195, 215, 240)
                                    }),
                                )
                                .fill(if selected {
                                    Color32::from_rgb(32, 118, 218)
                                } else {
                                    Color32::from_rgb(16, 30, 56)
                                })
                                .stroke(egui::Stroke::new(
                                    1.0,
                                    if selected {
                                        Color32::from_rgb(125, 205, 255)
                                    } else {
                                        Color32::from_rgba_unmultiplied(64, 130, 210, 60)
                                    },
                                ));
                                if ui.add_sized([pill_w4, 25.0], btn).clicked() && !selected {
                                    actions.push(Action::SetTopShape(sh));
                                }
                            }
                        });
                    });

                    ui.add_space(6.0);

                    // 4. Carte SAUVEGARDE D'ÉTAT RAPIDE
                    section_frame().show(ui, |ui| {
                        let inner_w = (card_width - 22.0).max(180.0);
                        ui.set_width(inner_w);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("ÉTATS DE SAUVEGARDE")
                                    .size(10.5)
                                    .strong()
                                    .color(Color32::from_rgb(110, 184, 250)),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("▶").clicked() {
                                    self.slot = if self.slot >= 9 { 1 } else { self.slot + 1 };
                                }
                                ui.label(RichText::new(format!("Slot {}", self.slot)).strong().size(12.0));
                                if ui.small_button("◀").clicked() {
                                    self.slot = if self.slot <= 1 { 9 } else { self.slot - 1 };
                                }
                                ui.label(RichText::new(saved.unwrap_or("vide")).size(11.0).weak());
                            });
                        });
                        ui.add_space(3.0);
                        let half = (inner_w - 6.0) / 2.0;
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if ui
                                .add_sized([half, 27.0], egui::Button::new("Sauvegarder (F5)"))
                                .clicked()
                            {
                                actions.push(Action::SaveState);
                            }
                            if ui
                                .add_enabled(
                                    saved.is_some(),
                                    egui::Button::new("Charger (F8)").min_size(Vec2::new(half, 27.0)),
                                )
                                .clicked()
                            {
                                actions.push(Action::LoadState);
                            }
                        });
                    });

                    ui.add_space(6.0);

                    // 5. Grille 2 colonnes d'options & outils
                    let col_w = (card_width - 8.0) / 2.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if ui.add_sized([col_w, 29.0], egui::Button::new("⚙  Réglages")).clicked() {
                            self.settings_open = true;
                        }
                        if ui
                            .add_sized([col_w, 29.0], egui::Button::new("✨  HD & Cheats"))
                            .on_hover_text("Activer / désactiver No Outlines (HD), mode 60 FPS Turbo et codes Action Replay.")
                            .clicked()
                        {
                            self.cheats_open = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if ui.add_sized([col_w, 29.0], egui::Button::new("⛶  Plein écran (F11)")).clicked() {
                            actions.push(Action::Fullscreen);
                        }
                        if ui.add_sized([col_w, 29.0], egui::Button::new("🌐  Multijoueur")).clicked() {
                            self.multiplayer_open = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if ui.add_sized([col_w, 29.0], egui::Button::new("↻  Redémarrer")).clicked() {
                            actions.push(Action::Reset);
                        }
                        if let Some(pid) = program_id {
                            if ui.add_sized([col_w, 29.0], egui::Button::new("🎛  Profil du jeu")).clicked() {
                                self.game_settings = Some((pid, name.to_owned()));
                            }
                        } else if ui.add_sized([col_w, 29.0], egui::Button::new("📷  Image QR")).clicked() {
                            actions.push(Action::ChooseCameraPicture);
                        }
                    });

                    ui.add_space(4.0);
                    ui.separator();

                    // 6. Barre inférieure compacte
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new("📷 QR / Caméra").size(11.5))
                            .on_hover_text("Choisir une image (ex: QR Code Base Secrète) pour l'appareil photo 3DS.")
                            .clicked()
                        {
                            actions.push(Action::ChooseCameraPicture);
                        }
                        if ui
                            .button(RichText::new("📋 Diagnostic").size(11.5))
                            .clicked()
                        {
                            actions.push(Action::CopyReport);
                        }
                        if ui
                            .button(RichText::new("🏠 Bibliothèque").size(11.5))
                            .clicked()
                        {
                            actions.push(Action::Library);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let quit_btn = egui::Button::new(
                                RichText::new("⏻ Quitter").size(12.0).strong().color(Color32::from_rgb(255, 195, 195)),
                            )
                            .fill(Color32::from_rgba_unmultiplied(92, 24, 34, 210))
                            .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(235, 95, 110, 110)));
                            if ui.add(quit_btn).clicked() {
                                actions.push(Action::Quit);
                            }
                        });
                    });

                    for job in jobs.iter().filter(|job| !job.stage().finished()) {
                        ui.separator();
                        ui.label(&job.name);
                        progress(ui, job);
                    }
                    if waiting > 0 {
                        ui.label(RichText::new(format!("{waiting} more waiting to recompile")).weak());
                    }
                });
            });
        actions
    }"#;

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
