// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Native renderer setup, without an external Windows OpenGL driver.

pub fn native_options(viewport: egui::ViewportBuilder, software: bool) -> eframe::NativeOptions {
    #[cfg(windows)]
    {
        use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetupCreateNew};

        let mut setup = WgpuSetupCreateNew::without_display_handle();
        setup.instance_descriptor.backends =
            wgpu::Backends::from_env().unwrap_or(wgpu::Backends::DX12);
        // FXC is supplied by Windows. DXC would require another shipped DLL.
        setup
            .instance_descriptor
            .backend_options
            .dx12
            .shader_compiler = wgpu::Dx12Compiler::Fxc;
        if software {
            setup.instance_descriptor.backends = wgpu::Backends::DX12;
            setup.native_adapter_selector = Some(std::sync::Arc::new(|adapters, surface| {
                adapters
                    .iter()
                    .find(|adapter| {
                        adapter.get_info().device_type == wgpu::DeviceType::Cpu
                            && surface.is_none_or(|surface| adapter.is_surface_supported(surface))
                    })
                    .cloned()
                    .ok_or_else(|| "The Windows software renderer is not available.".into())
            }));
        }
        eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Wgpu,
            wgpu_options: WgpuConfiguration {
                wgpu_setup: setup.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
    #[cfg(not(windows))]
    {
        let _ = software;
        eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        }
    }
}
