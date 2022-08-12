//! Ciphertext type in the BFV encryption scheme.

use crate::{
	parameters::{BfvParameters, MultiplicationParameters},
	traits::TryConvertFrom,
	EvaluationKey, Plaintext,
};
use fhers_protos::protos::{bfv::Ciphertext as CiphertextProto, rq::Rq};
use itertools::{izip, Itertools};
use math::rq::{traits::TryConvertFrom as PolyTryConvertFrom, Poly, Representation};
use num_bigint::BigUint;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::{
	ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign},
	rc::Rc,
};

/// A ciphertext encrypting a plaintext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ciphertext {
	/// The parameters of the underlying BFV encryption scheme.
	pub(crate) par: Rc<BfvParameters>,

	/// The seed that generated the polynomial c1 in a fresh ciphertext.
	pub(crate) seed: Option<<ChaCha8Rng as SeedableRng>::Seed>,

	/// The ciphertext elements.
	pub(crate) c: Vec<Poly>,
}

impl Add<&Ciphertext> for &Ciphertext {
	type Output = Ciphertext;

	fn add(self, rhs: &Ciphertext) -> Ciphertext {
		debug_assert_eq!(self.par, rhs.par);
		assert_eq!(self.c.len(), rhs.c.len());
		let c = izip!(&self.c, &rhs.c)
			.map(|(c1i, c2i)| c1i + c2i)
			.collect_vec();
		Ciphertext {
			par: self.par.clone(),
			seed: None,
			c,
		}
	}
}

impl AddAssign<&Ciphertext> for Ciphertext {
	fn add_assign(&mut self, rhs: &Ciphertext) {
		debug_assert_eq!(self.par, rhs.par);
		assert_eq!(self.c.len(), rhs.c.len());
		izip!(&mut self.c, &rhs.c).for_each(|(c1i, c2i)| *c1i += c2i);
		self.seed = None
	}
}

impl Sub<&Ciphertext> for &Ciphertext {
	type Output = Ciphertext;

	fn sub(self, rhs: &Ciphertext) -> Ciphertext {
		assert_eq!(self.par, rhs.par);
		assert_eq!(self.c.len(), rhs.c.len());
		let c = izip!(&self.c, &rhs.c)
			.map(|(c1i, c2i)| c1i - c2i)
			.collect_vec();
		Ciphertext {
			par: self.par.clone(),
			seed: None,
			c,
		}
	}
}

impl SubAssign<&Ciphertext> for Ciphertext {
	fn sub_assign(&mut self, rhs: &Ciphertext) {
		debug_assert_eq!(self.par, rhs.par);
		assert_eq!(self.c.len(), rhs.c.len());
		izip!(&mut self.c, &rhs.c).for_each(|(c1i, c2i)| *c1i -= c2i);
		self.seed = None
	}
}

impl Neg for &Ciphertext {
	type Output = Ciphertext;

	fn neg(self) -> Ciphertext {
		let c = self.c.iter().map(|c1i| -c1i).collect_vec();
		Ciphertext {
			par: self.par.clone(),
			seed: None,
			c,
		}
	}
}

impl MulAssign<&Plaintext> for Ciphertext {
	fn mul_assign(&mut self, rhs: &Plaintext) {
		assert_eq!(self.par, rhs.par);
		self.c.iter_mut().for_each(|ci| *ci *= &rhs.poly_ntt);
		self.seed = None
	}
}

impl Mul<&Plaintext> for &Ciphertext {
	type Output = Ciphertext;

	fn mul(self, rhs: &Plaintext) -> Self::Output {
		let c = self.c.iter().map(|c1i| c1i * &rhs.poly_ntt).collect_vec();
		Ciphertext {
			par: self.par.clone(),
			seed: None,
			c,
		}
	}
}

#[allow(dead_code)]
fn print_poly(s: &str, p: &Poly) {
	println!("{} = {:?}", s, Vec::<BigUint>::from(p))
}

/// Multiply two ciphertext and relinearize.
fn mul_internal(
	ct0: &Ciphertext,
	ct1: &Ciphertext,
	ek: &EvaluationKey,
	mp: &MultiplicationParameters,
) -> Result<Ciphertext, String> {
	if !ek.supports_relinearization() {
		return Err("The evaluation key does not support relinearization".to_string());
	}
	if ct0.par != ct1.par {
		return Err("Incompatible parameters".to_string());
	}
	if ct0.par.ciphertext_moduli.len() == 1 {
		return Err("Parameters do not allow for multiplication".to_string());
	}
	if ct0.c.len() != 2 || ct1.c.len() != 2 {
		return Err("Multiplication can only be performed on ciphertexts of size 2".to_string());
	}

	// Extend
	let mut now = std::time::SystemTime::now();
	let c00 = mp.extender_self.scale(&ct0.c[0], false)?;
	let c01 = mp.extender_self.scale(&ct0.c[1], false)?;
	let c10 = mp.extender_other.scale(&ct1.c[0], false)?;
	let c11 = mp.extender_other.scale(&ct1.c[1], false)?;
	println!("Extend: {:?}", now.elapsed().unwrap());

	// Multiply
	now = std::time::SystemTime::now();
	let mut c0 = &c00 * &c10;
	let mut c1 = &c00 * &c11;
	c1 += &(&c01 * &c10);
	let mut c2 = &c01 * &c11;
	c0.change_representation(Representation::PowerBasis);
	c1.change_representation(Representation::PowerBasis);
	c2.change_representation(Representation::PowerBasis);
	println!("Multiply: {:?}", now.elapsed().unwrap());

	// Scale
	// TODO: This should be faster??
	now = std::time::SystemTime::now();
	let mut c0 = mp.down_scaler.scale(&c0, false)?;
	let mut c1 = mp.down_scaler.scale(&c1, false)?;
	let c2 = mp.down_scaler.scale(&c2, false)?;
	println!("Scale: {:?}", now.elapsed().unwrap());

	// Relinearize
	now = std::time::SystemTime::now();
	c0.change_representation(Representation::Ntt);
	c1.change_representation(Representation::Ntt);
	ek.relinearizes(&mut c0, &mut c1, &c2)?;
	println!("Relinearize: {:?}", now.elapsed().unwrap());

	Ok(Ciphertext {
		par: ct0.par.clone(),
		seed: None,
		c: vec![c0, c1],
	})
}

/// Multiply two ciphertext and relinearize.
pub fn mul(ct0: &Ciphertext, ct1: &Ciphertext, ek: &EvaluationKey) -> Result<Ciphertext, String> {
	mul_internal(ct0, ct1, ek, &ct0.par.mul_1_params)
}

/// Multiply two ciphertext and relinearize.
pub fn mul2(ct0: &Ciphertext, ct1: &Ciphertext, ek: &EvaluationKey) -> Result<Ciphertext, String> {
	mul_internal(ct0, ct1, ek, &ct0.par.mul_2_params)
}

// pub fn inner_sum(ct: &Ciphertext, isk: &InnerSumKey) -> Result<Ciphertext, String> {

// }

/// Conversions from and to protobuf.
impl From<&Ciphertext> for CiphertextProto {
	fn from(ct: &Ciphertext) -> Self {
		let mut proto = CiphertextProto::new();
		for i in 0..ct.c.len() - 1 {
			proto.c.push(Rq::from(&ct.c[i]))
		}
		if let Some(seed) = ct.seed {
			proto.seed = seed.to_vec()
		} else {
			proto.c.push(Rq::from(&ct.c[ct.c.len() - 1]))
		}
		proto
	}
}

impl TryConvertFrom<&CiphertextProto> for Ciphertext {
	type Error = String;

	fn try_convert_from(
		value: &CiphertextProto,
		par: &Rc<BfvParameters>,
	) -> Result<Self, Self::Error> {
		if value.c.is_empty() || (value.c.len() == 1 && value.seed.is_empty()) {
			return Err("Not enough polynomials".to_string());
		}
		let mut seed = None;

		let mut c = Vec::with_capacity(value.c.len() + 1);
		for cip in &value.c {
			let mut ci = Poly::try_convert_from(cip, &par.ctx, None)?;
			unsafe { ci.allow_variable_time_computations() }
			c.push(ci)
		}

		if !value.seed.is_empty() {
			let try_seed = <ChaCha8Rng as SeedableRng>::Seed::try_from(value.seed.clone());
			if try_seed.is_err() {
				return Err("Invalid seed".to_string());
			}
			seed = try_seed.ok();
			let mut c1 = Poly::random_from_seed(&par.ctx, Representation::Ntt, seed.unwrap());
			unsafe { c1.allow_variable_time_computations() }
			c.push(c1)
		}

		Ok(Ciphertext {
			par: par.clone(),
			seed,
			c,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::{mul, mul2};
	use crate::{
		traits::{Decoder, Decryptor, Encoder, Encryptor, TryConvertFrom},
		BfvParameters, Ciphertext, Encoding, EvaluationKeyBuilder, Plaintext, SecretKey,
	};
	use fhers_protos::protos::bfv::Ciphertext as CiphertextProto;
	use std::rc::Rc;

	#[test]
	fn test_add() {
		let ntests = 100;
		for params in [
			Rc::new(BfvParameters::default(1)),
			Rc::new(BfvParameters::default(2)),
		] {
			for _ in 0..ntests {
				let a = params.plaintext.random_vec(params.degree());
				let b = params.plaintext.random_vec(params.degree());
				let mut c = a.clone();
				params.plaintext.add_vec(&mut c, &b);

				let sk = SecretKey::random(&params);

				for encoding in [Encoding::Poly, Encoding::Simd] {
					let pt_a =
						Plaintext::try_encode(&a as &[u64], encoding.clone(), &params).unwrap();
					let pt_b =
						Plaintext::try_encode(&b as &[u64], encoding.clone(), &params).unwrap();

					let mut ct_a = sk.encrypt(&pt_a).unwrap();
					let ct_b = sk.encrypt(&pt_b).unwrap();
					let ct_c = &ct_a + &ct_b;
					ct_a += &ct_b;

					let pt_c = sk.decrypt(&ct_c).unwrap();
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone()).unwrap(), c);
					let pt_c = sk.decrypt(&ct_a).unwrap();
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone()).unwrap(), c);
				}
			}
		}
	}

	#[test]
	fn test_sub() {
		for params in [
			Rc::new(BfvParameters::default(1)),
			Rc::new(BfvParameters::default(2)),
		] {
			let ntests = 100;
			for _ in 0..ntests {
				let a = params.plaintext.random_vec(params.degree());
				let b = params.plaintext.random_vec(params.degree());
				let mut c = a.clone();
				params.plaintext.sub_vec(&mut c, &b);

				let sk = SecretKey::random(&params);

				for encoding in [Encoding::Poly, Encoding::Simd] {
					let pt_a =
						Plaintext::try_encode(&a as &[u64], encoding.clone(), &params).unwrap();
					let pt_b =
						Plaintext::try_encode(&b as &[u64], encoding.clone(), &params).unwrap();

					let mut ct_a = sk.encrypt(&pt_a).unwrap();
					let ct_b = sk.encrypt(&pt_b).unwrap();
					let ct_c = &ct_a - &ct_b;
					ct_a -= &ct_b;

					let pt_c = sk.decrypt(&ct_c).unwrap();
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone()).unwrap(), c);
					let pt_c = sk.decrypt(&ct_a).unwrap();
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone()).unwrap(), c);
				}
			}
		}
	}

	#[test]
	fn test_neg() {
		for params in [
			Rc::new(BfvParameters::default(1)),
			Rc::new(BfvParameters::default(2)),
		] {
			let ntests = 100;
			for _ in 0..ntests {
				let a = params.plaintext.random_vec(params.degree());
				let mut c = a.clone();
				params.plaintext.neg_vec(&mut c);

				let sk = SecretKey::random(&params);
				for encoding in [Encoding::Poly, Encoding::Simd] {
					let pt_a =
						Plaintext::try_encode(&a as &[u64], encoding.clone(), &params).unwrap();

					let ct_a = sk.encrypt(&pt_a).unwrap();
					let ct_c = -&ct_a;

					let pt_c = sk.decrypt(&ct_c).unwrap();
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone()).unwrap(), c);
				}
			}
		}
	}

	#[test]
	fn test_scalar_mul() {
		for params in [
			Rc::new(BfvParameters::default(1)),
			Rc::new(BfvParameters::default(2)),
		] {
			let ntests = 100;
			for _ in 0..ntests {
				let a = params.plaintext.random_vec(params.degree());
				let b = params.plaintext.random_vec(params.degree());

				let sk = SecretKey::random(&params);
				for encoding in [Encoding::Poly, Encoding::Simd] {
					let mut c = vec![0u64; params.degree()];
					match encoding {
						Encoding::Poly => {
							for i in 0..params.degree() {
								for j in 0..params.degree() {
									if i + j >= params.degree() {
										c[(i + j) % params.degree()] = params.plaintext.sub(
											c[(i + j) % params.degree()],
											params.plaintext.mul(a[i], b[j]),
										);
									} else {
										c[i + j] = params
											.plaintext
											.add(c[i + j], params.plaintext.mul(a[i], b[j]));
									}
								}
							}
						}
						Encoding::Simd => {
							c = a.clone();
							params.plaintext.mul_vec(&mut c, &b);
						}
					}

					let pt_a =
						Plaintext::try_encode(&a as &[u64], encoding.clone(), &params).unwrap();
					let pt_b =
						Plaintext::try_encode(&b as &[u64], encoding.clone(), &params).unwrap();

					let mut ct_a = sk.encrypt(&pt_a).unwrap();
					let ct_c = &ct_a * &pt_b;
					ct_a *= &pt_b;

					let pt_c = sk.decrypt(&ct_c).unwrap();
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone()).unwrap(), c);
					let pt_c = sk.decrypt(&ct_a).unwrap();
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone()).unwrap(), c);
				}
			}
		}
	}

	#[test]
	fn test_mul() -> Result<(), String> {
		let par = Rc::new(BfvParameters::default(2));
		for _ in 0..50 {
			// We will encode `values` in an Simd format, and check that the product is computed correctly.
			let values = par.plaintext.random_vec(par.polynomial_degree);
			let mut expected = values.clone();
			par.plaintext.mul_vec(&mut expected, &values);

			let sk = SecretKey::random(&par);
			let ek = EvaluationKeyBuilder::new(&sk)
				.enable_relinearization()
				.build()?;
			let pt = Plaintext::try_encode(&values as &[u64], Encoding::Simd, &par)?;

			let ct1 = sk.encrypt(&pt)?;
			let ct2 = sk.encrypt(&pt)?;
			let ct3 = mul(&ct1, &ct2, &ek)?;

			println!("Noise: {}", unsafe { sk.measure_noise(&ct3)? });
			let pt = sk.decrypt(&ct3)?;
			assert_eq!(Vec::<u64>::try_decode(&pt, Encoding::Simd)?, expected);
		}
		Ok(())
	}

	#[test]
	fn test_mul2() -> Result<(), String> {
		let ntests = 100;
		let par = Rc::new(BfvParameters::default(2));
		for _ in 0..ntests {
			// We will encode `values` in an Simd format, and check that the product is computed correctly.
			let values = par.plaintext.random_vec(par.polynomial_degree);
			let mut expected = values.clone();
			par.plaintext.mul_vec(&mut expected, &values);

			let sk = SecretKey::random(&par);
			let ek = EvaluationKeyBuilder::new(&sk)
				.enable_relinearization()
				.build()?;
			let pt = Plaintext::try_encode(&values as &[u64], Encoding::Simd, &par)?;

			let ct1 = sk.encrypt(&pt)?;
			let ct2 = sk.encrypt(&pt)?;
			let ct3 = mul2(&ct1, &ct2, &ek)?;

			println!("Noise: {}", unsafe { sk.measure_noise(&ct3)? });
			let pt = sk.decrypt(&ct3)?;
			assert_eq!(Vec::<u64>::try_decode(&pt, Encoding::Simd)?, expected);
		}
		Ok(())
	}

	#[test]
	fn test_proto_conversion() -> Result<(), String> {
		for params in [
			Rc::new(BfvParameters::default(1)),
			Rc::new(BfvParameters::default(2)),
		] {
			let sk = SecretKey::random(&params);
			let v = params.plaintext.random_vec(8);
			let pt = Plaintext::try_encode(&v as &[u64], Encoding::Simd, &params)?;
			let ct = sk.encrypt(&pt)?;
			let ct_proto = CiphertextProto::from(&ct);
			assert_eq!(ct, Ciphertext::try_convert_from(&ct_proto, &params)?)
		}
		Ok(())
	}
}
