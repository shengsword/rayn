use crate::camera::Camera;
use crate::material::MaterialHandle;
use crate::math::{f32x4, OrthonormalBasis, Wat3, Wec3};
use crate::ray::{Ray, WRay};

use bumpalo::collections::Vec as BumpVec;
use bumpalo::Bump;

pub trait Hitable: Send + Sync {
    fn hit(&self, rays: &WRay, t_ranges: ::std::ops::Range<f32x4>) -> f32x4;
    // return 0 if occluded, 1 if not
    fn occluded(&self, start: Wec3, end: Wec3, time: f32x4) -> f32x4;
    /// err is a function that takes a point and returns the error bound
    /// at that point based on a screen-space projection (i.e. computes pixel size
    /// at that point).
    fn get_shading_info(
        &self,
        hits: WHit,
        primary: bool,
        camera: &dyn Camera,
    ) -> (MaterialHandle, WShadingPoint);
}

#[derive(Clone, Copy)]
pub struct WShadingPoint {
    pub ray: WRay,
    pub t: f32x4,
    pub point: Wec3,
    pub offset_by: f32x4,
    pub normal: Wec3,
    pub basis: Wat3,
}

impl WShadingPoint {
    pub fn new(hit: WHit, point: Wec3, offset_by: f32x4, normal: Wec3) -> Self {
        WShadingPoint {
            ray: hit.ray,
            t: hit.t,
            point,
            offset_by,
            normal,
            basis: normal.get_orthonormal_basis(),
        }
    }

    pub fn create_rays(&self, dir: Wec3) -> WRay {
        let mut ray = self.ray;
        ray.origin = self.point + self.normal * self.normal.dot(dir).signum() * self.offset_by;
        ray.dir = dir;
        ray
    }
}

#[derive(Clone, Copy)]
pub struct Hit {
    pub ray: Ray,
    pub t: f32,
}

#[derive(Clone, Copy)]
pub struct WHit {
    pub ray: WRay,
    pub t: f32x4,
}

impl WHit {
    #[inline]
    pub fn point(&self) -> Wec3 {
        self.ray.point_at(self.t)
    }
}

impl From<[Hit; 4]> for WHit {
    fn from(hits: [Hit; 4]) -> Self {
        let ray = WRay::from([hits[0].ray, hits[1].ray, hits[2].ray, hits[3].ray]);
        let t = f32x4::from([hits[0].t, hits[1].t, hits[2].t, hits[3].t]);
        Self { ray, t }
    }
}

pub struct HitStore<'bump> {
    hits: BumpVec<'bump, BumpVec<'bump, Hit>>,
}

impl<'bump> HitStore<'bump> {
    pub fn from_hitable_store(bump: &'bump Bump, hitable_store: &HitableStore) -> Self {
        let mut hits = BumpVec::with_capacity_in(hitable_store.len(), bump);
        for _ in 0..hitable_store.len() {
            hits.push(BumpVec::new_in(bump))
        }
        Self { hits }
    }

    pub unsafe fn add_hit(&mut self, obj_id: usize, hit: Hit) {
        self.hits.get_unchecked_mut(obj_id).push(hit);
    }

    pub fn process_hits(
        &mut self,
        hitables: &HitableStore,
        wintersections: &mut BumpVec<'_, (MaterialHandle, WShadingPoint)>,
        primary: bool,
        camera: &dyn Camera,
    ) {
        let total_hits = self
            .hits
            .iter_mut()
            .map(|hits| {
                while hits.len() % 4 != 0 {
                    hits.push(Hit {
                        ray: Ray::new_invalid(),
                        t: 0.0,
                    })
                }
                hits.len()
            })
            .sum::<usize>();

        wintersections.reserve(total_hits / 4);

        for (obj_id, hits) in self.hits.iter_mut().enumerate() {
            for hits in hits[0..].chunks_exact(4) {
                // Safe because we just assured that every window will have exactly
                // 4 members.
                let hits = WHit::from(unsafe {
                    [
                        *hits.get_unchecked(0),
                        *hits.get_unchecked(1),
                        *hits.get_unchecked(2),
                        *hits.get_unchecked(3),
                    ]
                });
                wintersections.push(
                    unsafe { hitables.get_unchecked(obj_id) }
                        .get_shading_info(hits, primary, camera),
                );
            }
        }
    }

    pub fn reset(&mut self) {
        for shading_points in self.hits.iter_mut() {
            shading_points.clear();
        }
    }
}

pub struct HitableStore(Vec<Box<dyn Hitable>>);

impl HitableStore {
    pub fn new() -> Self {
        HitableStore(Vec::new())
    }

    pub fn push<H: Hitable + 'static>(&mut self, hitable: H) {
        self.0.push(Box::new(hitable))
    }
}

impl ::std::ops::Deref for HitableStore {
    type Target = Vec<Box<dyn Hitable>>;

    fn deref(&self) -> &Vec<Box<dyn Hitable>> {
        &self.0
    }
}

impl HitableStore {
    pub fn test_occluded(&self, start: Wec3, end: Wec3, time: f32x4) -> f32x4 {
        self.iter().fold(f32x4::ONE, |acc, hitable| {
            acc * hitable.occluded(start, end, time)
        })
    }

    pub fn add_hits(
        &self,
        ray: WRay,
        t_ranges: ::std::ops::Range<f32x4>,
        hit_store: &mut HitStore,
    ) {
        let (ids, dists) = self.iter().enumerate().fold(
            ([std::usize::MAX; 4], t_ranges.end),
            |acc, (hitable_id, hitable)| {
                let (mut closest_ids, mut closest) = acc;

                let t = hitable.hit(&ray, t_ranges.start..closest);

                for ((t, closest), closest_id) in t
                    .as_ref()
                    .iter()
                    .zip(closest.as_mut().iter_mut())
                    .zip(closest_ids.iter_mut())
                {
                    if *t < *closest {
                        *closest = *t;
                        *closest_id = hitable_id;
                    }
                }

                (closest_ids, closest)
            },
        );

        let rays: [Ray; 4] = ray.into();
        let dists = dists.as_ref();

        for ((id, ray), t) in ids.iter().zip(rays.iter()).zip(dists.iter()) {
            if *id < std::usize::MAX && ray.valid {
                unsafe {
                    hit_store.add_hit(*id, Hit { ray: *ray, t: *t });
                }
            }
        }
    }
}
